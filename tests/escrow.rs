use anchor_lang::{prelude::*, solana_program::instruction::Instruction, system_program, InstructionData};
use anchor_spl::token::{self};
use solana_program_test::*;
use solana_sdk::{
    clock::Clock,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Test setup
struct TestContext {
    program_id: Pubkey,
    context: ProgramTestContext,
    initializer: Keypair,
    recipient: Keypair,
    arbiter: Keypair,
    mint_authority: Keypair,
    mint: Pubkey,
    initializer_token_account: Pubkey,
    recipient_token_account: Pubkey,
}

impl TestContext {
    async fn new() -> Self {
        let program_id = escrow::id();
        let mut program_test = ProgramTest::new("escrow", program_id, processor!(escrow::entry));
        let mut context = program_test.start_with_context().await;

        let initializer = Keypair::new();
        let recipient = Keypair::new();
        let arbiter = Keypair::new();
        let mint_authority = Keypair::new();

        let mint =
            Self::create_mint(&mut context, &mint_authority.pubkey(), &mint_authority).await;

        let initializer_token_account = Self::create_token_account(
            &mut context,
            &mint,
            &initializer.pubkey(),
            &mint_authority,
            100,
        )
        .await;

        let recipient_token_account = Self::create_token_account(
            &mut context,
            &mint,
            &recipient.pubkey(),
            &mint_authority,
            0,
        )
        .await;

        Self {
            program_id,
            context,
            initializer,
            recipient,
            arbiter,
            mint_authority,
            mint,
            initializer_token_account,
            recipient_token_account,
        }
    }

    async fn create_mint(
        context: &mut ProgramTestContext,
        authority: &Pubkey,
        payer: &Keypair,
    ) -> Pubkey {
        let mint = Keypair::new();
        let rent = context.banks_client.get_rent().await.unwrap();
        let mint_rent = rent.minimum_balance(spl_token::state::Mint::LEN);

        let tx = Transaction::new_signed_with_payer(
            &[
                solana_sdk::system_instruction::create_account(
                    &context.payer.pubkey(),
                    &mint.pubkey(),
                    mint_rent,
                    spl_token::state::Mint::LEN as u64,
                    &spl_token::id(),
                ),
                spl_token::instruction::initialize_mint(
                    &spl_token::id(),
                    &mint.pubkey(),
                    authority,
                    None,
                    0,
                )
                .unwrap(),
            ],
            Some(&context.payer.pubkey()),
            &[&context.payer, &mint],
            context.last_blockhash,
        );
        context.banks_client.process_transaction(tx).await.unwrap();
        mint.pubkey()
    }

    async fn create_token_account(
        context: &mut ProgramTestContext,
        mint: &Pubkey,
        owner: &Pubkey,
        mint_authority: &Keypair,
        amount: u64,
    ) -> Pubkey {
        let token_account = Keypair::new();
        let rent = context.banks_client.get_rent().await.unwrap();
        let token_rent = rent.minimum_balance(spl_token::state::Account::LEN);

        let tx = Transaction::new_signed_with_payer(
            &[
                solana_sdk::system_instruction::create_account(
                    &context.payer.pubkey(),
                    &token_account.pubkey(),
                    token_rent,
                    spl_token::state::Account::LEN as u64,
                    &spl_token::id(),
                ),
                spl_token::instruction::initialize_account(
                    &spl_token::id(),
                    &token_account.pubkey(),
                    mint,
                    owner,
                )
                .unwrap(),
                spl_token::instruction::mint_to(
                    &spl_token::id(),
                    mint,
                    &token_account.pubkey(),
                    &mint_authority.pubkey(),
                    &[],
                    amount,
                )
                .unwrap(),
            ],
            Some(&context.payer.pubkey()),
            &[&context.payer, &token_account, mint_authority],
            context.last_blockhash,
        );
        context.banks_client.process_transaction(tx).await.unwrap();
        token_account.pubkey()
    }

    async fn get_token_balance(&mut self, account: &Pubkey) -> u64 {
        let account_info = self
            .context
            .banks_client
            .get_account(*account)
            .await
            .unwrap()
            .unwrap();
        let token_account = spl_token::state::Account::unpack(&account_info.data).unwrap();
        token_account.amount
    }

    async fn get_account<T: anchor_lang::AccountDeserialize>(
        &mut self,
        address: &Pubkey,
    ) -> Option<T> {
        self.context
            .banks_client
            .get_account(*address)
            .await
            .unwrap()
            .map(|acc| T::try_deserialize(&mut acc.data.as_slice()).unwrap())
    }
}

#[tokio::test]
async fn test_initialize_and_withdraw() {
    let mut test_harness = TestContext::new().await;

    let amount = 50;
    let timeout =
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64 + 10;

    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.recipient.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );

    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.recipient.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize { amount, timeout }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    assert_eq!(
        test_harness
            .get_token_balance(&test_harness.initializer_token_account)
            .await,
        50
    );
    assert_eq!(test_harness.get_token_balance(&vault_pda).await, 50);

    let withdraw_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Withdraw {
            recipient: test_harness.recipient.pubkey(),
            recipient_deposit_token_account: test_harness.recipient_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Withdraw {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[withdraw_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.recipient],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    assert_eq!(
        test_harness
            .get_token_balance(&test_harness.recipient_token_account)
            .await,
        50
    );

    let escrow_account = test_harness.get_account::<escrow::Escrow>(&escrow_state_pda).await.unwrap();
    assert_eq!(escrow_account.status, escrow::EscrowStatus::Withdrawn);
}

#[tokio::test]
async fn test_initialize_and_refund() {
    let mut test_harness = TestContext::new().await;

    let amount = 50;
    let timeout = 1; // 1 second timeout for faster testing

    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.recipient.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );

    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.recipient.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize { amount, timeout }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let refund_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Refund {
            initializer: test_harness.initializer.pubkey(),
            initializer_refund_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Refund {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[refund_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    assert_eq!(
        test_harness
            .get_token_balance(&test_harness.initializer_token_account)
            .await,
        100
    );
    let escrow_account = test_harness.get_account::<escrow::Escrow>(&escrow_state_pda).await.unwrap();
    assert_eq!(escrow_account.status, escrow::EscrowStatus::Refunded);
}

#[tokio::test]
#[should_panic]
async fn test_initialize_with_zero_amount() {
    let mut test_harness = TestContext::new().await;
    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.recipient.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );
    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.recipient.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize {
            amount: 0,
            timeout: 10,
        }
        .data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness.context.banks_client.process_transaction(tx).await.unwrap();
}

#[tokio::test]
#[should_panic]
async fn test_initialize_with_self_as_recipient() {
    let mut test_harness = TestContext::new().await;
    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.initializer.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );
    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.initializer.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize {
            amount: 10,
            timeout: 10,
        }
        .data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness.context.banks_client.process_transaction(tx).await.unwrap();
}

#[tokio::test]
#[should_panic]
async fn test_withdraw_after_timeout() {
    let mut test_harness = TestContext::new().await;
    let amount = 50;
    let timeout = 1;

    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.recipient.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );

    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.recipient.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize { amount, timeout }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_secs(2)).await;

    let withdraw_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Withdraw {
            recipient: test_harness.recipient.pubkey(),
            recipient_deposit_token_account: test_harness.recipient_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Withdraw {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[withdraw_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.recipient],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();
}

#[tokio::test]
#[should_panic]
async fn test_refund_before_timeout() {
    let mut test_harness = TestContext::new().await;
    let amount = 50;
    let timeout = 10;

    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.recipient.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );

    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.recipient.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize { amount, timeout }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    let refund_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Refund {
            initializer: test_harness.initializer.pubkey(),
            initializer_refund_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Refund {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[refund_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();
}

#[tokio::test]
#[should_panic]
async fn test_withdraw_with_invalid_recipient() {
    let mut test_harness = TestContext::new().await;
    let amount = 50;
    let timeout = 10;

    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.recipient.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );

    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.recipient.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize { amount, timeout }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();

    let invalid_recipient = Keypair::new();

    let withdraw_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Withdraw {
            recipient: invalid_recipient.pubkey(),
            recipient_deposit_token_account: test_harness.recipient_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Withdraw {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[withdraw_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &invalid_recipient],
        test_harness.context.last_blockhash,
    );
    test_harness
        .context
        .banks_client
        .process_transaction(tx)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_cancel_escrow() {
    let mut test_harness = TestContext::new().await;
    let amount = 50;
    let timeout = 100; // Long timeout

    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.recipient.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );

    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.recipient.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize { amount, timeout }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness.context.banks_client.process_transaction(tx).await.unwrap();

    let cancel_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Cancel {
            initializer: test_harness.initializer.pubkey(),
            initializer_refund_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Cancel {}.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[cancel_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness.context.banks_client.process_transaction(tx).await.unwrap();

    assert_eq!(
        test_harness
            .get_token_balance(&test_harness.initializer_token_account)
            .await,
        100
    );
    let escrow_account = test_harness.get_account::<escrow::Escrow>(&escrow_state_pda).await.unwrap();
    assert_eq!(escrow_account.status, escrow::EscrowStatus::Cancelled);
}

#[tokio::test]
async fn test_resolve_by_arbiter_to_recipient() {
    let mut test_harness = TestContext::new().await;
    let amount = 50;
    let timeout = 100;

    let (escrow_state_pda, _) = Pubkey::find_program_address(
        &[
            b"escrow",
            test_harness.initializer.pubkey().as_ref(),
            test_harness.recipient.pubkey().as_ref(),
        ],
        &test_harness.program_id,
    );

    let (vault_pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_state_pda.as_ref()],
        &test_harness.program_id,
    );

    let init_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::Initialize {
            initializer: test_harness.initializer.pubkey(),
            recipient: test_harness.recipient.pubkey(),
            arbiter: test_harness.arbiter.pubkey(),
            mint: test_harness.mint,
            initializer_deposit_token_account: test_harness.initializer_token_account,
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            system_program: system_program::id(),
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::Initialize { amount, timeout }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[init_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.initializer],
        test_harness.context.last_blockhash,
    );
    test_harness.context.banks_client.process_transaction(tx).await.unwrap();

    let resolve_ix = Instruction {
        program_id: test_harness.program_id,
        accounts: escrow::accounts::ResolveByArbiter {
            arbiter: test_harness.arbiter.pubkey(),
            escrow_state: escrow_state_pda,
            vault: vault_pda,
            recipient_deposit_token_account: test_harness.recipient_token_account,
            initializer_refund_token_account: test_harness.initializer_token_account,
            token_program: token::ID,
        }
        .to_account_metas(None),
        data: escrow::instruction::ResolveByArbiter { release_to_recipient: true }.data(),
    };

    let tx = Transaction::new_signed_with_payer(
        &[resolve_ix],
        Some(&test_harness.context.payer.pubkey()),
        &[&test_harness.context.payer, &test_harness.arbiter],
        test_harness.context.last_blockhash,
    );
    test_harness.context.banks_client.process_transaction(tx).await.unwrap();

    assert_eq!(
        test_harness
            .get_token_balance(&test_harness.recipient_token_account)
            .await,
        50
    );
    let escrow_account = test_harness.get_account::<escrow::Escrow>(&escrow_state_pda).await.unwrap();
    assert_eq!(escrow_account.status, escrow::EscrowStatus::Withdrawn);
}

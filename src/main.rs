use solana_account_decoder::{UiAccountEncoding, UiDataSliceConfig};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType},
};
use solana_program::instruction::Instruction;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    signer::{keypair::Keypair, Signer},
    transaction::Transaction,
};

#[derive(serde::Deserialize)]
struct Env {
    rpc_url: url::Url,
    vault_pubkey: String,
    program_pubkey: String,
    signer_keypair: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let env = envy::from_env::<Env>()?;
    let vault: Pubkey = env.vault_pubkey.parse()?;
    let program: Pubkey = env.program_pubkey.parse()?;
    let signer: Keypair = Keypair::from_base58_string(&env.signer_keypair);
    let rpc_client = RpcClient::new(env.rpc_url.to_string());

    let memcmp = RpcFilterType::Memcmp(Memcmp::new(
        8,                                            // offset
        MemcmpEncodedBytes::Base64(env.vault_pubkey), // encoded bytes
    ));
    let config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::DataSize(80), memcmp]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            data_slice: Some(UiDataSliceConfig {
                offset: 40, // 8 + 32
                length: 41, // 32 + 1 + 8
            }),
            commitment: Some(CommitmentConfig::processed()),
            min_context_slot: None,
        },
        with_context: Some(false),
    };
    let users = rpc_client
        .get_program_accounts_with_config(&program, config)
        .await?;

    let vault_data = rpc_client.get_account_data(&vault).await?;
    let mint_pubkey = unsafe {
        let buff = *(vault_data[8 + 32..8 + 32 + 32].as_ptr() as *const [u8; 32]);
        Pubkey::new_from_array(buff)
    };
    let mint_decimals = vault_data[8 + 32 + 32..8 + 32 + 32 + 1][0];
    // let total_amount = unsafe {
    //     let buff =
    //         *(vault_data[8 + 32 + 32 + 1 + 1..8 + 32 + 32 + 1 + 1 + 8].as_ptr() as *const [u8; 8]);
    //     u64::from_le_bytes(buff)
    // };
    let signer_token_account =
        spl_associated_token_account::get_associated_token_address(&signer.pubkey(), &mint_pubkey);

    for user in users {
        let mut instructions: Vec<Instruction> = vec![];

        let owner = unsafe {
            let buff = *(user.1.data[0..32].as_ptr() as *const [u8; 32]);
            Pubkey::new_from_array(buff)
        };
        let amount = unsafe {
            let buff = *(user.1.data[32 + 1..32 + 1 + 8].as_ptr() as *const [u8; 8]);
            u64::from_le_bytes(buff)
        };
        let user_token_account =
            spl_associated_token_account::get_associated_token_address(&owner, &mint_pubkey);
        let user_token_account_info = rpc_client.get_account(&user_token_account).await?;

        if user_token_account_info.lamports == 0 {
            instructions.push(
                spl_associated_token_account::instruction::create_associated_token_account(
                    &user_token_account,
                    &signer.pubkey(),
                    &mint_pubkey,
                    &spl_token::ID,
                ),
            );
        }
        instructions.push(spl_token::instruction::transfer_checked(
            &spl_token::ID,
            &signer_token_account,
            &mint_pubkey,
            &user_token_account,
            &signer.pubkey(),
            &[&signer.pubkey()],
            amount.checked_div(100).unwrap(), // 1% of staked amount
            mint_decimals,
        )?);

        let recent_blockhash = rpc_client.get_latest_blockhash().await?;
        let transaction = Transaction::new_signed_with_payer(
            instructions.as_ref(),
            Some(&signer.pubkey()),
            &[&signer],
            recent_blockhash,
        );
        rpc_client
            .send_and_confirm_transaction_with_spinner(&transaction)
            .await?;
    }

    Ok(())
}

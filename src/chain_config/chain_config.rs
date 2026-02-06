
use std::collections::HashMap;

use alloy::primitives::FixedBytes;
use alloy::providers::{Provider, ProviderBuilder};
use uuid::Uuid;
use crate::config::config::AppConfig;
use crate::state_models::models::ProviderConnection;
use crate::error::error::AppError;
use once_cell::sync::Lazy;
use sha2::{ Sha256};
use hmac::{Hmac, Mac};
use alloy_signer_local::LocalSigner;
type HmacSha256 = Hmac<Sha256>;

fn uuid_to_seed(uuid: Uuid, secret: &[u8]) -> FixedBytes<32> {
    let mut mac = HmacSha256::new_from_slice(secret)
        .expect("HMAC can take key of any size");

    mac.update(uuid.as_bytes());

    let result = mac.finalize().into_bytes();

    let bytes: [u8; 32] = result
        .as_slice()
        .try_into()
        .expect("HMAC-SHA256 output is 32 bytes");

    FixedBytes::from(bytes)
}




pub static CHAIN_RPC : Lazy<HashMap<String , Vec< &'static str>>>= Lazy ::new(||{
    let mut map = HashMap::new();

    map.insert("bitlayer_testnet".to_string() , vec!["https://testnet-rpc.bitlayer.org"]);
    map.insert("base_sepolia".to_string(), vec!["https://base-sepolia.g.alchemy.com/v2/XcjcviYCCB6UB3T5uwSk1dIIA-sbrA-p"]);
    map.insert("base_mainnet".to_string(), vec!["https://base-mainnet.g.alchemy.com/v2/je8NBeGlxHuC1m6VCHB93"]);
    map.insert("test_base_mainnet".to_string(), vec!["https://go.getblock.asia/399e2b8bd3fa44f3a9d05d3390ae43e2"]);
    map.insert("umi_devnet".to_string(), vec!["https://devnet.uminetwork.com/evm"]);
    map
});


pub async fn create_provider(chain: &String , user_id: Uuid) -> Result<ProviderConnection, AppError> {

    let wallet_generation_secret = AppConfig::from_env().unwrap().wallet_generation_secret;
        
    let secret = hex::decode(wallet_generation_secret.trim())
            .map_err(|_| AppError::InternalError
                ("Invalid WALLET_GENERATION_SECRET".into()))?;

    let seed = uuid_to_seed(user_id, &secret);

    let signer = LocalSigner::from_bytes(&seed)
            .map_err(|_| AppError::InternalError("Signer creation failed".into()))?;

    let rpc_list = CHAIN_RPC
        .get(chain)
        .ok_or_else(|| AppError::InternalError(format!("No RPCs found for chain {}", chain)))?;


    for rpc in rpc_list {

        if let Ok(rpc_url) = rpc.parse() {

            let provider = ProviderBuilder::new()
                .with_cached_nonce_management()
                .wallet(signer.clone())
                .connect_http(rpc_url);

            match provider.get_chain_id().await {
                Ok(_) => {
                    // println!("✅ Connected to RPC: {}", rpc);
                    return Ok(ProviderConnection(provider));
                }
                Err(err) => {
                    eprintln!("⚠️ Failed to connect to RPC {}: {}", rpc, err);
                    continue;
                }
            }
        }
    }

    Err(AppError::InternalError("All RPC endpoints failed".to_string()))
}



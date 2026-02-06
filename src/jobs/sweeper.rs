use std::{ str::FromStr, time::Duration};

use alloy::{
    primitives::{ Address, U256}, providers::Provider,  sol
};
use rust_decimal::{Decimal, prelude::FromPrimitive};
use sea_orm::{
    ActiveModelTrait, ActiveValue::{ Set}, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, TransactionTrait
};
use uuid::Uuid;

use tokio::time::sleep;
use crate::{
    chain_config::chain_config::create_provider, config::config::AppConfig, entities::{ prelude::UserWallet, user_wallet}, error::error::AppError, jobs::index::{MAX_RETRIES, RETRY_BACKOFF, between_cycles_cleanup}, state_models::models::DbConnection,  tokens::tokens::TOKENS, utils::{free_wallet::mark_wallet_free, token_decimals::get_token_decimals, update_deposit::upsert_user_balance_and_receipt},
};


sol!(
    #[sol(rpc)]
    ERC20,
    "src/utils/abi/ERC20.json"
);



pub async fn sweep_wallet(
    worker_id: u64,
    db: &DbConnection,
) -> Result<(), AppError> {


            let txn = db.0.begin().await.map_err(AppError::DbError)?;

            // Select up to 100 pending requests and lock them for this worker
            let user_wallets = UserWallet::find()
                .filter(user_wallet::Column::Status.eq("SWEEPABLE"))
                .filter(user_wallet::Column::ActiveGas.lt(0.1))
                .order_by_asc(user_wallet::Column::CreatedAt)
                .limit(100)
                .lock_exclusive() // <- row-level exclusive lock
                .all(&txn)
                .await
                .map_err(AppError::DbError)?;

            // Mark them as IN_PROGRESS so other workers skip them
            for req in &user_wallets {
                let mut active: user_wallet::ActiveModel = req.clone().into();
                active.status = Set("SWEEP_IN_PROGRESS".to_string());
                active.update(&txn).await.map_err(AppError::DbError)?;
            }

            // Commit so locks are released and status is updated
            txn.commit().await.map_err(AppError::DbError)?;

    for user_wallet in user_wallets{

        let mut retries = 0;

        loop {
            let result = process_single_request(worker_id, user_wallet.id, &db).await;

            match result {
            Ok(_) => {
                // success, move to next request
                break;
            },

            Err(e) if retries < MAX_RETRIES => {
                retries += 1;
                eprintln!("Retry {} for request {}: {}", retries, user_wallet.id, e);
                sleep(RETRY_BACKOFF * retries).await; // backoff
            },

            Err(e) => {
                    //  Reset back to PENDING for retry in next cycle

                    eprintln!("Failed to process request {} after {} retries: {}", user_wallet.id, retries, e);

                    let mut complete_request: user_wallet::ActiveModel = user_wallet.clone().into();
                        complete_request.status = Set("FREE".to_string());
                        complete_request
                        .update(&db.0)
                        .await
                        .map_err(AppError::DbError)?;
                        break;
                
            }
        }
            sleep(Duration::from_millis(150)).await;
            between_cycles_cleanup(&db).await;
            tokio::task::yield_now().await;
        }
    
    
    
    }
    Ok(())
}



async fn process_single_request(
    worker_id: u64,
    user_wallet_id: Uuid,
    db: &DbConnection,
)->Result<(), AppError> {

    let config = AppConfig::from_env()?;
    let txn = db.0.begin().await.map_err(AppError::DbError)?;
    let master_wallet_address = Address::from_str(config.master_wallet_address.as_str()).unwrap() ;
    let pending_wallet = UserWallet::find_by_id(user_wallet_id)
    .lock_exclusive()
    .one(&txn)
    .await?;

    if pending_wallet.is_none() {
        return Ok(()); // already deleted or processed
    }

    let pending_wallet = pending_wallet.unwrap();

    println!("Wallet : {} in Worker : {}", pending_wallet.wallet_address, worker_id);
    let wallet_address: Address = pending_wallet.wallet_address.parse().map_err(|e|{
        eprintln!("Invalid wallet address {:?}",e);
        AppError::BadRequest("Invalid wallet address".to_string())
    } )?;

    let user_id = pending_wallet.user_id;

    for (chain_name , tokens) in TOKENS.iter(){

        println!("Checking Chain {} on Wallet {}", chain_name, wallet_address);

        let mut usdc_token_balance ;
        let mut usdt_token_balance;

        let provider = create_provider(&chain_name , user_id).await.map_err(|e| {
            eprintln!("Cannot create provider on  {:?}: {:?}", chain_name, e);
            AppError::InternalError(format!("Provider error: {e}"))
        })?;

        let gas_price = provider.0.get_gas_price().await.map_err(|e|{
                    eprintln!("Error Cannot get gas price {:?}: {:?}", wallet_address, e);
                    AppError::InternalError(format!("Error Cannot get gas price : {e}"))
            } )?;

        for (token_name , token_address) in tokens {

            let erc20 = ERC20::new(*token_address, &provider.0);
            
            let gas_balance  = provider.0.get_balance(wallet_address).await.map_err(|e| AppError::InternalError(format!("Cannot fetch native balance: {e}")))?;

            if token_name ==  &"USDC"  {

                let decimals = get_token_decimals(&provider.0, token_address.clone()).await?;

                usdc_token_balance  = erc20.balanceOf(wallet_address).call().await.map_err(|e|{   
                        eprintln!("Error fetching USDC balance for {:?}: {:?}", wallet_address, e);
                        AppError::InternalError(format!("Provider error: {e}"))
                    })?;

                if usdc_token_balance > U256::ZERO {
                        let call = erc20.transfer(master_wallet_address, U256::from(1));

                        let usdc_transfer_gas = call.from(master_wallet_address)
                            .estimate_gas()
                            .await.map_err(|e|{
                            eprintln!("Error Cannot estimate gas {:?}: {:?}", wallet_address, e);
                            AppError::InternalError(format!("Error Cannot estimate gas : {e}"))
                        } )?;

                        let mut  minimum_gas = U256::from(usdc_transfer_gas) * U256::from(gas_price);

                        // +20% buffer
                        let buffer = minimum_gas / U256::from(5);

                        minimum_gas = minimum_gas + buffer;
                    if gas_balance >= minimum_gas { 
                            let tx_hash = erc20.transfer(master_wallet_address, usdc_token_balance).send().await.map_err(|e|{
                            eprintln!("Error: Cannot sent {:?}: {:?}", master_wallet_address , e);
                            AppError::InternalError(format!("Provider error: {e}"))
                    })?.watch().await.map_err(|e|{
                            eprintln!("Error : Cannot get receipt  {:?}: {:?}", master_wallet_address , e);
                            AppError::InternalError(format!("Provider error: {e}"))
                    })?;

                    println!("Transaction Hash : {} Token:{}", tx_hash, token_name);
                    let usdc_decimal = u256_to_decimal(usdc_token_balance, decimals)?;

                    upsert_user_balance_and_receipt(
                                &txn,
                                user_id,
                                &wallet_address.to_string(),
                                &token_address.to_string(),
                                &chain_name.to_string(),
                                usdc_decimal,
                                &tx_hash.to_string(),
                            ).await?;
                    
                        }else{
                            eprintln!("No Mininum gas Gas: {} Minimum Gas :{} Chain:{} Wallet:{} ", gas_balance, minimum_gas, chain_name, wallet_address);
                        }
                }else{
                    println!("No token Balance {} Token :{} Chain:{} Wallet:{} ", usdc_token_balance, token_name, chain_name, wallet_address);
                }

            }else {

                let decimals = get_token_decimals(&provider.0, token_address.clone()).await?;
                usdt_token_balance  = erc20.balanceOf(wallet_address).call().await.map_err(|e|{
                    eprintln!("Error fetching USDT balance for {:?}: {:?}", wallet_address, e);
                    AppError::InternalError(format!("Provider error: {e}"))
                } )?;


            if usdt_token_balance > U256::ZERO  {
                
                let call = erc20.transfer(master_wallet_address, U256::from(1));

                    let usdt_transfer_gas = call.from(master_wallet_address)
                        .estimate_gas()
                        .await.map_err(|e|{
                        eprintln!("Error Cannot estimate gas {:?}: {:?}", wallet_address, e);
                        AppError::InternalError(format!("Error Cannot estimate gas : {e}"))
                    } )?;

                    let mut  minimum_gas = U256::from(usdt_transfer_gas) * U256::from(gas_price);

                    // +20% buffer
                    let buffer = minimum_gas / U256::from(5);

                minimum_gas = minimum_gas + buffer;

                if gas_balance >= minimum_gas {

                    let tx_hash = erc20.transfer(master_wallet_address, usdt_token_balance).send().await.map_err(|e|{
                                eprintln!("Error: Cannot sent Wallet:{:?} Gas : {:?} error: {:?}", master_wallet_address , gas_balance, e );
                                AppError::InternalError(format!("Provider error: {e}"))
                        })?.watch().await.map_err(|e|{
                                eprintln!("Error : Cannot get receipt  {:?}: {:?}", master_wallet_address , e);
                                AppError::InternalError(format!("Provider error: {e}"))
                        })?;
                    
                    println!("Transaction Hash : {} Token:{}", tx_hash, token_name);

                    let usdt_decimal = u256_to_decimal(usdt_token_balance, decimals)?;

                    upsert_user_balance_and_receipt(
                            &txn,
                            user_id,
                            &wallet_address.to_string(),
                            &token_address.to_string(),
                            &chain_name.to_string(),
                            usdt_decimal,
                            &tx_hash.to_string(),
                        )
                        .await?;
                    }else{
                        eprintln!("No Mininum gas Gas : {} Minimum Gas :{} Chain:{} Wallet:{} ", gas_balance, minimum_gas, chain_name, wallet_address);
                    }
                }else{
                    println!("No token Balance {} Token :{} Chain:{} Wallet:{} ", usdt_token_balance, token_name, chain_name, wallet_address);
                }
            }


        }
    }

    let mut complete_request: user_wallet::ActiveModel = pending_wallet.into();
                        complete_request.status = Set("FREE".to_string());
                        complete_request
                        .update(&txn)
                        .await
                        .map_err(AppError::DbError)?;

    txn.commit().await.map_err(AppError::DbError)?;


    Ok(())
}



fn u256_to_decimal(amount: U256, decimals: u8) -> Result<Decimal, AppError> {
    let base = Decimal::from_i128(10_i128.pow(decimals as u32))
        .ok_or_else(|| AppError::InternalError("Decimal overflow".into()))?;

    let value = Decimal::from_str(&amount.to_string())
        .map_err(|e| AppError::InternalError(format!("Decimal parse error: {e}")))?;

    Ok(value / base)
}
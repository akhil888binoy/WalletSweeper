use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QuerySelect,  ActiveValue::{ Set}};
use uuid::Uuid;
use rust_decimal::Decimal;

use crate::{entities::{deposit_receipt, user_balance}, error::error::AppError};

pub async fn upsert_user_balance_and_receipt(
    txn: &sea_orm::DatabaseTransaction,
    user_id: Uuid,
    wallet_address: &str,
    token_address: &str,
    chain_name: &str,
    amount: Decimal,
    tx_hash: &str,
) -> Result<(), AppError> {

    let existing_balance = user_balance::Entity::find()
        .filter(user_balance::Column::Userid.eq(user_id))
        .filter(user_balance::Column::Token.eq(token_address))
        .filter(user_balance::Column::Chain.eq(chain_name))
        .lock_exclusive()
        .one(txn)
        .await
        .map_err(AppError::DbError)?;

    //  Update or Insert user_balance
    if let Some(record) = existing_balance {
        let mut active: user_balance::ActiveModel = record.into();
            active.balance = Set(active.balance.unwrap() + amount);
            active.updated_at = Set(chrono::Utc::now().into());
            active.update(txn).await.map_err(|e|{
                eprintln!("Error: Cannot update existing_balance {}",e);
                AppError::DbError(e)
        })?;

    } else {
        user_balance::ActiveModel {
            id: Set(Uuid::new_v4()),
            userid: Set(user_id),
            token: Set(token_address.to_string()),
            chain: Set(chain_name.to_string()),
            balance: Set(amount),
            created_at: Set(chrono::Utc::now().into()),
            updated_at: Set(chrono::Utc::now().into()),
        }
        .insert(txn)
        .await
        .map_err(AppError::DbError)?;
    }

    //  Insert deposit receipt (always append-only)
    deposit_receipt::ActiveModel {
        id: Set(Uuid::new_v4()),
        userid: Set(user_id.to_string()),
        user_address: Set(wallet_address.to_string()),
        token: Set(token_address.to_string()),
        chain: Set(chain_name.to_string()),
        amount: Set(amount),
        txn_hash: Set(tx_hash.to_string()),
        created_at: Set(chrono::Utc::now().into()),
        updated_at: Set(chrono::Utc::now().into()),
    }
    .insert(txn)
    .await
    .map_err(AppError::DbError)?;

    Ok(())
}

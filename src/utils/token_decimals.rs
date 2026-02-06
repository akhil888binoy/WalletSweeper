use alloy::{primitives::Address, providers::Provider, sol};

use crate::error::error::AppError;

sol!(
    #[sol(rpc)]
    ERC20,
    "src/utils/abi/ERC20.json"
);

pub async fn get_token_decimals<P: Provider>(
    provider: &P,
    token: Address,
) -> Result<u8, AppError> {
    if token.is_zero() {
        return Ok(18); // ETH
    }

    let erc20 = ERC20::new(token, provider);
    let decimals = erc20.decimals().call().await
        .map_err(|e| AppError::InternalError(format!("Decimals call failed: {}", e)))?;

    // Verify contract
    let code = provider.get_code_at(token).await
        .map_err(|e| AppError::InternalError(format!("Code check failed: {}", e)))?;
    
    if code.is_empty() {
        return Err(AppError::BadRequest("Not a contract".to_string()));
    }

    Ok(decimals)
}
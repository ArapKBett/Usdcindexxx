use anyhow::Result;
use chrono::{DateTime, Utc};
use solana_client::rpc_client::{RpcClient, GetSignaturesForAddressConfig};
use solana_sdk::pubkey::Pubkey;
use solana_transaction_status::{
    EncodedTransaction, UiInstruction, UiParsedInstruction, UiTransactionEncoding,
};
use std::str::FromStr;
use warp::Filter;

const USDC_MINT_ADDRESS: &str = "Es9vMFrzaCERH16Cdv83hA5KaM6rDx8JEX5Rk3z3aZ9o";
const WALLET_ADDRESS: &str = "7cMEhpt9y3inBNVv8fNnuaEbx7hKHZnLvR1KWKKxuDDU";

async fn backfill_usdc_transfers() -> Result<String> {
    let rpc_url = "https://api.mainnet-beta.solana.com";
    let client = RpcClient::new(rpc_url.to_string());

    let wallet = Pubkey::from_str(WALLET_ADDRESS)?;

    let now = chrono::Utc::now();
    let cutoff_ts = now.timestamp() - 24 * 3600;

    let mut before_signature: Option<String> = None;
    let mut transfers = Vec::new();

    'outer: loop {
        let sigs = client.get_signatures_for_address_with_config(
            &wallet,
            GetSignaturesForAddressConfig {
                before: before_signature.clone(),
                limit: Some(1000),
                ..Default::default()
            },
        )?;

        if sigs.is_empty() {
            break;
        }

        for sig_info in &sigs {
            let block_time_opt = sig_info.block_time;
            if block_time_opt.is_none() {
                // Skip if no block time
                continue;
            }
            let block_time = block_time_opt.unwrap();

            if block_time < cutoff_ts {
                break 'outer;
            }

            // Fetch transaction with parsed JSON encoding
            let tx = client.get_transaction_with_config(
                &sig_info.signature.parse()?,
                solana_client::rpc_config::RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::JsonParsed),
                    commitment: None,
                    max_supported_transaction_version: None,
                },
            )?;

            let enc_tx = &tx.transaction.transaction;

            // Only handle JsonParsed transactions
            let instructions = match enc_tx {
                EncodedTransaction::Json(parsed_tx) => &parsed_tx.message.instructions,
                _ => continue,
            };

            for ix in instructions {
                if let UiInstruction::Parsed(ui_parsed) = ix {
                    // UiParsedInstruction is an enum, handle variants
                    match ui_parsed {
                        UiParsedInstruction::Parsed(parsed) => {
                            if parsed.program != "spl-token" {
                                continue;
                            }

                            let instruction_type = parsed
                                .parsed
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if instruction_type != "transfer" && instruction_type != "transferChecked" {
                                continue;
                            }

                            let info = match parsed.parsed.get("info") {
                                Some(i) => i,
                                None => continue,
                            };

                            // Check mint address
                            if let Some(mint) = info.get("mint").and_then(|v| v.as_str()) {
                                if mint != USDC_MINT_ADDRESS {
                                    continue;
                                }
                            }

                            let source = info.get("source").and_then(|v| v.as_str());
                            let destination = info.get("destination").and_then(|v| v.as_str());

                            let amount_str = info
                                .get("amount")
                                .and_then(|v| v.as_str())
                                .or_else(|| {
                                    info.get("tokenAmount")
                                        .and_then(|token_amount| token_amount.get("amount").and_then(|v| v.as_str()))
                                })
                                .unwrap_or("0");

                            let amount_u64 = amount_str.parse::<u64>().unwrap_or(0);
                            if amount_u64 == 0 {
                                continue;
                            }

                            let amount = amount_u64 as f64 / 1_000_000f64; // USDC has 6 decimals

                            let direction = if let Some(src) = source {
                                if src == WALLET_ADDRESS {
                                    "sent"
                                } else if let Some(dest) = destination {
                                    if dest == WALLET_ADDRESS {
                                        "received"
                                    } else {
                                        continue;
                                    }
                                } else {
                                    continue;
                                }
                            } else {
                                continue;
                            };

                            let date = DateTime::<Utc>::from_utc(
                                chrono::NaiveDateTime::from_timestamp(block_time, 0),
                                Utc,
                            );

                            transfers.push(format!(
                                "{} | {}{:.6} USDC | {}",
                                date.to_rfc3339(),
                                if direction == "sent" { "-" } else { "+" },
                                amount,
                                direction,
                            ));
                        }
                        _ => continue,
                    }
                }
            }
        }

        before_signature = sigs.last().map(|s| s.signature.clone());
    }

    transfers.sort();
    Ok(transfers.join("\n"))
}

async fn handle_backfill() -> Result<impl warp::Reply, warp::Rejection> {
    match backfill_usdc_transfers().await {
        Ok(data) => Ok(warp::reply::with_status(data, warp::http::StatusCode::OK)),
        Err(e) => Ok(warp::reply::with_status(
            format!("Error: {}", e),
            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
        )),
    }
}

#[tokio::main]
async fn main() {
    let route = warp::path("backfill").and(warp::get()).and_then(handle_backfill);

    // Listen on 0.0.0.0:10000 (Render default)
    warp::serve(route).run(([0, 0, 0, 0], 10000)).await;
                            }
                                         

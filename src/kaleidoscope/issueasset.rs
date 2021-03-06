use bitcoin::network::constants::Network;
use bitcoin::OutPoint;
use bitcoin::util::hash::Sha256dHash;
use chain::tx_builder::{build_issuance_tx, raw_tx_commit_to};
use chain::wallet::*;
use clap::ArgMatches;
use database::Database;
use jsonrpc;
use jsonrpc::client::Client;
use kaleidoscope::{Config, RGBSubCommand};
use rgb::contract::Contract;
use rgb::proof::OutputEntry;
use rgb::proof::Proof;
use std::collections::HashMap;

pub struct IssueAsset {}

impl<'a> RGBSubCommand<'a> for IssueAsset {
    fn run(matches: &'a ArgMatches<'a>, config: &Config, database: &mut Database, client: &mut Client) -> Result<(), jsonrpc::Error> {
        let unspent_utxos = rpc_list_unspent(client).unwrap();
        let mut unspent_utxos_outpoints: Vec<&OutPoint> = unspent_utxos.keys().collect();
        // filter out the UTXOs with proof attached to them
        let unspent_utxos_outpoints: Vec<&&OutPoint> = unspent_utxos_outpoints
            .iter()
            .filter(|outpoint| database.get_proofs_for(*outpoint).len() == 0)
            .collect();

        const FEE: u64 = 3000;

        if unspent_utxos_outpoints.len() < 2 {
            eprintln!("Necessary number of UTXOs not reached!");
            return Err(jsonrpc::Error::NoErrorOrResult);
        }

        let issuance_utxo: OutPoint = match matches.value_of("issuance_utxo") {
            Some(utxo) => {
                let parts: Vec<&str> = utxo.split(":").collect();

                OutPoint {
                    txid: Sha256dHash::from_hex(parts[0]).unwrap(),
                    vout: parts[1].parse().unwrap(),
                }
            }
            None => *unspent_utxos_outpoints[0].clone()
        };

        let initial_owner_utxo: OutPoint = match matches.value_of("initial_owner_utxo") {
            Some(utxo) => {
                let parts: Vec<&str> = utxo.split(":").collect();

                OutPoint {
                    txid: Sha256dHash::from_hex(parts[0]).unwrap(),
                    vout: parts[1].parse().unwrap(),
                }
            }
            None => *unspent_utxos_outpoints[1].clone()
        };

        let network = match matches.value_of("network").unwrap() {
            "mainnet" => Network::Bitcoin,
            "testnet" => Network::Testnet,
            "regtest" => Network::Regtest,
            _ => panic!("Invalid network")
        };

        // -------------------------------------

        let burn_address = rpc_getnewaddress(client).unwrap();

        let contract = Contract {
            title: matches.value_of("title").unwrap().to_string(),
            total_supply: matches.value_of("total_supply").unwrap().parse().unwrap(),
            burn_address,
            network,
            issuance_utxo,
            initial_owner_utxo,
        };

        println!("Asset ID: {}", contract.get_asset_id());

        let change_address = rpc_getnewaddress(client).unwrap();
        let change_amount = unspent_utxos.get(&contract.issuance_utxo).unwrap() - FEE;

        let mut commit_tx_outputs = HashMap::new();
        commit_tx_outputs.insert(change_address, change_amount);

        let issuance_tx = build_issuance_tx(&contract, &commit_tx_outputs);
        let issuance_tx = rpc_sign_transaction(client, &issuance_tx).unwrap();

        println!("Spending the issuance_utxo {} in {}", contract.issuance_utxo, issuance_tx.txid());

        // -------------------------------------

        let root_proof = Proof::new(
            vec![contract.initial_owner_utxo.clone()],
            vec![],
            vec![OutputEntry::new(contract.get_asset_id(), contract.total_supply, 0)],
            Some(&contract));

        let root_proof_change_address = rpc_getnewaddress(client).unwrap();
        let root_proof_change_amounts = unspent_utxos.get(&contract.initial_owner_utxo).unwrap() - FEE;

        let mut proof_commit_tx_outputs = HashMap::new();
        proof_commit_tx_outputs.insert(root_proof_change_address, root_proof_change_amounts);

        let root_proof_tx = raw_tx_commit_to(
            &root_proof,
            vec![contract.initial_owner_utxo.clone()],
            &proof_commit_tx_outputs,
        );
        let root_proof_tx = rpc_sign_transaction(client, &root_proof_tx).unwrap();

        println!("Spending the initial_owner_utxo {} in {}", contract.initial_owner_utxo, root_proof_tx.txid());

        database.save_proof(&root_proof, &root_proof_tx.txid());

        rpc_broadcast(client, &issuance_tx)?;
        rpc_broadcast(client, &root_proof_tx)?;

        Ok(())
    }
}
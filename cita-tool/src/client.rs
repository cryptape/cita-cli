use std::str::FromStr;
use std::default::Default;
use std::{io, str, u64};

use tokio_core::reactor::Core;
use hyper::{self, Body, Client as HyperClient, Method, Request, Uri};
use serde_json;
use futures::{Future, Stream, future::JoinAll, future::join_all};
use super::{JsonRpcParams, JsonRpcResponse, ParamsValue, PrivKey, ResponseValue, Transaction};
use hex::{decode, encode};
use protobuf::Message;
use uuid::Uuid;

const CITA_BLOCK_BUMBER: &str = "cita_blockNumber";
const CITA_GET_META_DATA: &str = "cita_getMetaData";

/// Jsonrpc client, Only to one chain
#[derive(Debug)]
pub struct Client {
    id: u64,
    core: Core,
    chain_id: Option<u32>,
    private_key: Option<PrivKey>,
}

impl Client {
    /// Create a client for CITA
    pub fn new() -> io::Result<Self> {
        let core = Core::new()?;
        Ok(Client {
            id: 0,
            core: core,
            chain_id: None,
            private_key: None,
        })
    }

    /// Set chain id
    pub fn set_chain_id(mut self, chain_id: u32) -> Self {
        self.chain_id = Some(chain_id);
        self
    }

    /// Set private key
    pub fn set_private_key(mut self, private_key: PrivKey) -> Self {
        self.private_key = Some(private_key);
        self
    }

    /// Get private key
    pub fn private_key(&self) -> &PrivKey {
        self.private_key.as_ref().unwrap()
    }

    /// Send requests
    pub fn send_request(
        &mut self,
        urls: Vec<&str>,
        params: JsonRpcParams,
    ) -> Result<Vec<JsonRpcResponse>, ()> {
        let reqs = self.make_requests_with_all_url(urls, params);

        Ok(self.run(reqs))
    }

    /// Send multiple params to one node
    pub fn send_request_with_multiple_params<T: Iterator<Item = JsonRpcParams>>(
        &mut self,
        url: &str,
        params: T,
    ) -> Result<Vec<JsonRpcResponse>, ()> {
        let reqs = self.make_requests_with_params_list(url, params);

        Ok(self.run(reqs))
    }

    fn make_requests_with_all_url(
        &mut self,
        urls: Vec<&str>,
        params: JsonRpcParams,
    ) -> JoinAll<Vec<Box<Future<Item = hyper::Chunk, Error = hyper::error::Error>>>> {
        self.id = self.id.overflowing_add(1).0;
        let params = params.insert("id", ParamsValue::Int(self.id));
        let client = HyperClient::new(&self.core.handle());
        let mut reqs = Vec::new();
        urls.iter().for_each(|url| {
            let uri = Uri::from_str(url).unwrap();
            let mut req: Request<Body> = Request::new(Method::Post, uri);
            req.set_body(serde_json::to_string(&params).unwrap());
            let future: Box<Future<Item = hyper::Chunk, Error = hyper::error::Error>> =
                Box::new(client.request(req).and_then(|res| res.body().concat2()));
            reqs.push(future);
        });
        join_all(reqs)
    }

    fn make_requests_with_params_list<T: Iterator<Item = JsonRpcParams>>(
        &mut self,
        url: &str,
        params: T,
    ) -> JoinAll<Vec<Box<Future<Item = hyper::Chunk, Error = hyper::error::Error>>>> {
        let client = HyperClient::new(&self.core.handle());
        let mut reqs = Vec::new();
        params
            .map(|param| {
                self.id = self.id.overflowing_add(1).0;
                param.insert("id", ParamsValue::Int(self.id))
            })
            .for_each(|param| {
                let uri = Uri::from_str(&url).unwrap();
                let mut req: Request<Body> = Request::new(Method::Post, uri);
                req.set_body(serde_json::to_string(&param).unwrap());
                let future: Box<
                    Future<Item = hyper::Chunk, Error = hyper::error::Error>,
                > = Box::new(client.request(req).and_then(|res| res.body().concat2()));
                reqs.push(future);
            });

        join_all(reqs)
    }

    /// Constructing a UnverifiedTransaction hex string
    /// If you want to create a contract, set address to ""
    pub fn generate_transaction(
        &mut self,
        code: &str,
        address: String,
        current_height: u64,
    ) -> String {
        let data = decode(code).unwrap();

        let mut tx = Transaction::new();
        tx.set_data(data);
        // Create a contract if the target address is empty
        tx.set_to(address);
        tx.set_nonce(encode(Uuid::new_v4().as_bytes()));
        tx.set_valid_until_block(current_height + 88);
        tx.set_quota(1000000);
        tx.set_chain_id(self.chain_id.expect("Please set chain id"));
        encode(
            tx.sign(*self.private_key())
                .take_transaction_with_sig()
                .write_to_bytes()
                .unwrap(),
        )
    }

    /// Get chain id
    pub fn get_chain_id(&mut self, url: &str) -> u32 {
        if self.chain_id.is_some() {
            self.chain_id.unwrap()
        } else {
            if let Some(ResponseValue::Map(mut value)) = self.get_metadata(url).result() {
                match value.remove("chainId").unwrap() {
                    ParamsValue::Int(chain_id) => {
                        self.chain_id = Some(chain_id as u32);
                        return chain_id as u32;
                    }
                    _ => return 0,
                }
            } else {
                0
            }
        }
    }

    /// Start run
    fn run(
        &mut self,
        reqs: JoinAll<Vec<Box<Future<Item = hyper::Chunk, Error = hyper::error::Error>>>>,
    ) -> Vec<JsonRpcResponse> {
        let responses = self.core.run(reqs).unwrap();
        responses
            .into_iter()
            .map(|response| serde_json::from_slice::<JsonRpcResponse>(&response).unwrap())
            .collect::<Vec<JsonRpcResponse>>()
    }
}


/// High level jsonrpc call
///
/// [Documentation](https://cryptape.github.io/cita/zh/usage-guide/rpc/index.html)
///
/// JSONRPC methods:
///   * net_peerCount
///   * cita_blockNumber
///   * cita_sendTransaction
///   * cita_getBlockByHash
///   * cita_getBlockByNumber
///   * eth_getTransactionReceipt
///   * eth_getLogs
///   * eth_call
///   * cita_getTransaction
///   * eth_getTransactionCount
///   * eth_getCode
///   * eth_getAbi
///   * eth_getBalance
///   * eth_newFilter
///   * eth_newBlockFilter
///   * eth_uninstallFilter
///   * eth_getFilterChanges
///   * eth_getFilterLogs
///   * cita_getTransactionProof
///   * cita_getMetaData
pub trait ClientExt {
    /// net_peerCount: Get network peer count
    fn get_net_peer_count(&mut self, url: &str) -> u32;
    /// cita_blockNumber: Get current height
    fn get_block_number(&mut self, url: &str) -> Option<u64>;
    /// cita_sendTransaction: Send a transaction return transaction hash
    fn send_transaction(&mut self, url: &str) -> Result<String, String>;
    /// cita_getBlockByHash: Get block by hash
    fn get_block_by_hash(&mut self, url: &str) -> JsonRpcResponse;
    /// cita_getBlockByNumber: Get block by number
    fn get_block_by_number(&mut self, url: &str) -> JsonRpcResponse;
    /// eth_getTransactionReceipt: Get transaction receipt
    fn get_transaction_receipt(&mut self, url: &str) -> JsonRpcResponse;
    /// eth_getLogs: Get logs
    fn get_logs(&mut self, url: &str) -> JsonRpcResponse;
    /// eth_call: (readonly, will not save state change)
    fn call(&mut self, url: &str) -> JsonRpcResponse;
    /// cita_getTransaction: Get transaction by hash
    fn get_transaction(&mut self, url: &str) -> JsonRpcResponse;
    /// eth_getTransactionCount: Get transaction count of an account
    fn get_transaction_count(&mut self, url: &str) -> u64;
    /// eth_getCode: Get the code of a contract
    fn get_code(&mut self, url: &str) -> String;
    /// eth_getAbi: Get the ABI of a contract
    fn get_abi(&mut self, url: &str) -> String;
    /// eth_getBalance: Get the balance of a contract (TODO: return U256)
    fn get_balance(&mut self, url: &str) -> u64;
    /// eth_newFilter:
    fn new_filter(&mut self, url: &str) -> u64;
    /// eth_newBlockFilter:
    fn new_block_filter(&mut self, url: &str) -> u64;
    /// eth_uninstallFilter: Uninstall a filter by its id
    fn uninstall_filter(&mut self, url: &str) -> bool;
    /// eth_getFilterChanges: Get filter changes
    fn get_filter_changes(&mut self, url: &str) -> JsonRpcResponse;
    /// eth_getFilterLogs: Get filter logs
    fn get_filter_logs(&mut self, url: &str) -> JsonRpcResponse;
    /// cita_getTransactionProof: Get proof of a transaction
    fn get_transaction_proof(&mut self, url: &str) -> JsonRpcResponse;
    /// cita_getMetaData: Get metadata
    fn get_metadata(&mut self, url: &str) -> JsonRpcResponse;
}

impl ClientExt for Client {
    fn get_net_peer_count(&mut self, _url: &str) -> u32 {
        Default::default()
    }

    fn get_block_number(&mut self, url: &str) -> Option<u64> {
        let params = JsonRpcParams::new().insert(
            "method",
            ParamsValue::String(String::from(CITA_BLOCK_BUMBER)),
        );
        let result = self.send_request(vec![url], params).unwrap().pop().unwrap();

        if let ResponseValue::Singe(ParamsValue::String(height)) = result.result().unwrap() {
            Some(u64::from_str_radix(&remove_0x(height), 16).unwrap())
        } else {
            None
        }
    }

    fn send_transaction(&mut self, _url: &str) -> Result<String, String> {
        Ok(String::from("tx_hash"))
    }

    fn get_block_by_hash(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn get_block_by_number(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn get_transaction_receipt(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn get_logs(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn call(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn get_transaction(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn get_transaction_count(&mut self, _url: &str) -> u64 {
        0
    }

    fn get_code(&mut self, _url: &str) -> String {
        Default::default()
    }

    fn get_abi(&mut self, _url: &str) -> String {
        Default::default()
    }

    fn get_balance(&mut self, _url: &str) -> u64 {
        0
    }

    fn new_filter(&mut self, _url: &str) -> u64 {
        0
    }

    fn new_block_filter(&mut self, _url: &str) -> u64 {
        0
    }

    fn uninstall_filter(&mut self, _url: &str) -> bool {
        false
    }

    fn get_filter_changes(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn get_filter_logs(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn get_transaction_proof(&mut self, _url: &str) -> JsonRpcResponse {
        Default::default()
    }

    fn get_metadata(&mut self, url: &str) -> JsonRpcResponse {
        let params = JsonRpcParams::new()
            .insert(
                "params",
                ParamsValue::List(vec![ParamsValue::String(String::from("latest"))]),
            )
            .insert(
                "method",
                ParamsValue::String(String::from(CITA_GET_META_DATA)),
            );
        self.send_request(vec![url], params).unwrap().pop().unwrap()
    }
}

fn remove_0x(hex: String) -> String {
    let tmp = hex.as_bytes();
    if tmp[..2] == b"0x"[..] {
        String::from_utf8(tmp[2..].to_vec()).unwrap()
    } else {
        hex.clone()
    }
}

//! Contract configuration for Polymarket
//!
//! This module contains contract addresses and configuration for different
//! networks and environments.

use std::collections::HashMap;

/// Contract configuration for a specific network
#[derive(Debug, Clone)]
pub struct ContractConfig {
    pub exchange: String,
    pub collateral: String,
    pub conditional_tokens: String,
}

/// Get contract configuration for a specific chain and risk setting
pub fn get_contract_config(chain_id: u64, neg_risk: bool) -> Option<ContractConfig> {
    match neg_risk {
        true => {
            if chain_id == 137 {
                return Some(ContractConfig {
                    exchange: "0xC5d563A36AE78145C45a50134d48A1215220f80a".to_owned(),
                    collateral: "0x2791bca1f2de4661ed88a30c99a7a9449aa84174".to_owned(),
                    conditional_tokens: "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045".to_owned(),
                });
            } else if chain_id == 80002 {
                return Some(ContractConfig {
                    exchange: "0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296".to_owned(),
                    collateral: "0x9c4e1703476e875070ee25b56a58b008cfb8fa78".to_owned(),
                    conditional_tokens: "0x69308FB512518e39F9b16112fA8d994F4e2Bf8bB".to_owned(),
                });
            }
            None
        }
        false => {
            if chain_id == 137 {
                return Some(ContractConfig {
                    exchange: "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E".to_owned(),
                    collateral: "0x2791Bca1f2de4661ED88A30C99a7a9449Aa84174".to_owned(),
                    conditional_tokens: "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045".to_owned(),
                });
            } else if chain_id == 80002 {
                return Some(ContractConfig {
                    exchange: "0xdFE02Eb6733538f8Ea35D585af8DE5958AD99E40".to_owned(),
                    collateral: "0x9c4e1703476e875070ee25b56a58b008cfb8fa78".to_owned(),
                    conditional_tokens: "0x69308FB512518e39F9b16112fA8d994F4e2Bf8bB".to_owned(),
                });
            }
            None
        }
    }
}

/// Network configuration
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub chain_id: u64,
    pub name: String,
    pub rpc_url: String,
    pub block_explorer: String,
    pub contracts: HashMap<String, ContractConfig>,
}

impl NetworkConfig {
    /// Get configuration for Polygon mainnet
    pub fn polygon_mainnet() -> Self {
        let mut contracts = HashMap::new();
        contracts.insert("standard".to_string(), get_contract_config(137, false).unwrap());
        contracts.insert("neg_risk".to_string(), get_contract_config(137, true).unwrap());

        Self {
            chain_id: 137,
            name: "Polygon Mainnet".to_string(),
            rpc_url: "https://polygon-rpc.com".to_string(),
            block_explorer: "https://polygonscan.com".to_string(),
            contracts,
        }
    }

    /// Get configuration for Polygon Mumbai testnet
    pub fn polygon_mumbai() -> Self {
        let mut contracts = HashMap::new();
        contracts.insert("standard".to_string(), get_contract_config(80002, false).unwrap());
        contracts.insert("neg_risk".to_string(), get_contract_config(80002, true).unwrap());

        Self {
            chain_id: 80002,
            name: "Polygon Mumbai".to_string(),
            rpc_url: "https://rpc-mumbai.maticvigil.com".to_string(),
            block_explorer: "https://mumbai.polygonscan.com".to_string(),
            contracts,
        }
    }

    /// Get contract configuration for this network
    pub fn get_contract(&self, risk_type: &str) -> Option<&ContractConfig> {
        self.contracts.get(risk_type)
    }
}

/// Global configuration
#[derive(Debug, Clone)]
pub struct GlobalConfig {
    pub networks: HashMap<u64, NetworkConfig>,
    pub default_network: u64,
}

impl GlobalConfig {
    /// Create default configuration
    pub fn new() -> Self {
        let mut networks = HashMap::new();
        networks.insert(137, NetworkConfig::polygon_mainnet());
        networks.insert(80002, NetworkConfig::polygon_mumbai());

        Self {
            networks,
            default_network: 137,
        }
    }

    /// Get network configuration
    pub fn get_network(&self, chain_id: u64) -> Option<&NetworkConfig> {
        self.networks.get(&chain_id)
    }

    /// Get default network
    pub fn default_network(&self) -> Option<&NetworkConfig> {
        self.networks.get(&self.default_network)
    }
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_config() {
        let config = get_contract_config(137, false);
        assert!(config.is_some());
        
        let config = config.unwrap();
        assert!(!config.exchange.is_empty());
        assert!(!config.collateral.is_empty());
        assert!(!config.conditional_tokens.is_empty());
    }

    #[test]
    fn test_network_config() {
        let polygon = NetworkConfig::polygon_mainnet();
        assert_eq!(polygon.chain_id, 137);
        assert_eq!(polygon.name, "Polygon Mainnet");
        
        let contract = polygon.get_contract("standard");
        assert!(contract.is_some());
    }

    #[test]
    fn test_global_config() {
        let config = GlobalConfig::new();
        assert_eq!(config.default_network, 137);
        
        let network = config.get_network(137);
        assert!(network.is_some());
    }
} 
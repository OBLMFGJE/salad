use eng_wasm::*;
use eng_wasm::{String, Vec, H160, H256, U256, eprint, decrypt, generate_key, SymmetricKey};
use eng_wasm_derive::eth_contract;
use eng_wasm_derive::pub_interface;
use enigma_crypto::hash::Keccak256;
use enigma_crypto::KeyPair;
use rustc_hex::ToHex;

#[eth_contract("ISalad.json")]
struct EthContract;

// State key name "mixer_eth_addr" holding eth address of Mixer contract
static MIXER_ETH_ADDR: &str = "mixer_eth_addr";
static ENCRYPTION_KEY: &str = "encryption_key";

const PUB_KEY_SIZE: usize = 64;
const UNIT256_SIZE: usize = 32;
const SIG_SIZE: usize = 65;
const ADDRESS_SIZE: usize = 20;

#[pub_interface]
trait ContractInterface {
    /// Constructor function that takes in MIXER_ETH_ADDR ethereum contract address
    fn construct(mixer_eth_addr: H160);

    fn get_pub_key() -> Vec<u8>;

    fn execute_deal(
        operator_address: H160,
        operator_nonce: U256,
        nb_recipients: U256,
        amount: U256,
        pub_keys: Vec<Vec<u8>>,
        enc_recipients: Vec<Vec<u8>>,
        senders: Vec<H160>,
        signatures: Vec<Vec<u8>>,
    ) -> Vec<H160>;

    fn verify_deposits(
        nb_recipients: U256,
        amount: U256,
        pub_keys: Vec<Vec<u8>>,
        enc_recipients: Vec<Vec<u8>>,
        senders: Vec<H160>,
        signatures: Vec<Vec<u8>>,
    ) -> bool;
}

struct Contract;

impl Contract {
    /// Read voting address of MIXER_ETH_ADDR contract
    fn get_mixer_eth_addr() -> String {
        read_state!(MIXER_ETH_ADDR).unwrap_or_default()
    }

    fn get_pkey() -> SymmetricKey {
        let key = read_state!(ENCRYPTION_KEY).unwrap();
        eprint!("Got key: {:?}", key);
        key
    }

    fn get_keypair() -> KeyPair {
        let key = Self::get_pkey();
        KeyPair::from_slice(&key).unwrap()
    }

    fn verify_signature(
        signature: [u8; SIG_SIZE],
        sender: &H160,
        amount: &U256,
        enc_recipient: &[u8],
        user_pubkey: &[u8; PUB_KEY_SIZE],
    ) -> H160 {
        eprint!("Verifying signature: {:?}", signature.to_vec());
        let mut message: Vec<u8> = Vec::new();
        message.extend_from_slice(b"\x19\x01");

        let mut domain_message: Vec<u8> = Vec::new();
        let eip712_domain_seperator = b"EIP712Domain(string name,string version,uint256 chainId)".keccak256().to_vec();
        let domain_name_hash = b"Salad Deposit".keccak256().to_vec();
        let domain_version_hash = b"1".keccak256().to_vec();
        // TODO: Pass as an argument
        let chain_id: usize = 50;
        let chain_id_hash = H256::from(&U256::from(chain_id)).keccak256().to_vec();
        domain_message.extend_from_slice(&eip712_domain_seperator);
        domain_message.extend_from_slice(&domain_name_hash);
        domain_message.extend_from_slice(&domain_version_hash);
        domain_message.extend_from_slice(&chain_id_hash);
        let domain_hash = domain_message.keccak256().to_vec();
        message.extend_from_slice(&domain_hash);

        let mut deposit_message: Vec<u8> = Vec::new();
        let deposit_seperator_hash = b"Deposit(address sender,uint256 amount,bytes encRecipient,bytes pubKey)".keccak256().to_vec();
        deposit_message.extend_from_slice(&deposit_seperator_hash);
        deposit_message.extend_from_slice(sender);
        deposit_message.extend_from_slice(&H256::from(amount));
        deposit_message.extend_from_slice(enc_recipient);
        deposit_message.extend_from_slice(user_pubkey);
        let deposit_hash = deposit_message.keccak256().to_vec();
        message.extend_from_slice(&deposit_hash);
        eprint!("The typed data message: {:?}", message);

        let sender_pubkey = KeyPair::recover(&message.keccak256().to_vec(), signature).unwrap();
        let mut sender_raw = [0u8; 20];
        sender_raw.copy_from_slice(&sender_pubkey.keccak256()[12..32]);
        let sender = H160::from(&sender_raw);
        eprint!("Recovered sender: {:?}", sender);
        sender
    }

    fn generate_deal_id(
        amount: &U256,
        participants: &Vec<H160>,
        operator_address: &H160,
        operator_nonce: &U256,
    ) -> H256 {
        let u32_prefix: [u8; 4] = [0; 4];
        let mut message: Vec<u8> = Vec::new();
        message.extend_from_slice(&u32_prefix);
        message.extend_from_slice(&UNIT256_SIZE.to_be_bytes());
        message.extend_from_slice(&H256::from(amount));
        message.extend_from_slice(&u32_prefix);
        message.extend_from_slice(&participants.len().to_be_bytes());
        for sender in participants.iter() {
            message.extend_from_slice(&u32_prefix);
            message.extend_from_slice(&ADDRESS_SIZE.to_be_bytes());
            message.extend_from_slice(sender);
        }
        message.extend_from_slice(&u32_prefix);
        message.extend_from_slice(&ADDRESS_SIZE.to_be_bytes());
        message.extend_from_slice(operator_address);
        message.extend_from_slice(&u32_prefix);
        message.extend_from_slice(&UNIT256_SIZE.to_be_bytes());
        message.extend_from_slice(&H256::from(operator_nonce));
        eprint!("The DealId message: {:?}", message);
        let mut hash_raw: [u8; 32] = [0; 32];
        hash_raw.copy_from_slice(&message.keccak256().to_vec());
        H256::from(&hash_raw)
    }

    fn verify_deposits_internal(
        nb_recipients: U256,
        amount: U256,
        pub_keys: Vec<Vec<u8>>,
        enc_recipients: Vec<Vec<u8>>,
        senders: Vec<H160>,
        signatures: Vec<Vec<u8>>,
    ) -> Vec<H160> {
        let mut recipients: Vec<H160> = Vec::new();
        let keypair = Self::get_keypair();
        for i in 0..nb_recipients.low_u64() as usize {
            eprint!("Decrypting recipient {}: {:?}", i, enc_recipients[i]);
            let mut user_pubkey = [0; PUB_KEY_SIZE];
            user_pubkey.copy_from_slice(&pub_keys[i]);
            eprint!("The user pubKey: {:?}", user_pubkey.to_vec());

            let shared_key = keypair.derive_key(&user_pubkey).unwrap();
            let plaintext = decrypt(&enc_recipients[i], &shared_key);
            eprint!("Successfully decrypted recipient {}", i);
            let recipient = H160::from(&plaintext[0..20]);

            let mut signature = [0; SIG_SIZE];
            signature.copy_from_slice(&signatures[i]);

            let sig_sender = Self::verify_signature(signature,
                                                    &senders[i],
                                                    &amount,
                                                    &enc_recipients[i],
                                                    &user_pubkey);
            if sig_sender != senders[i] {
                panic!(
                    "Invalid sender recovered from the signature: {:?} != {:?}",
                    sig_sender, senders[i]
                );
            }
            recipients.push(recipient);
        }
        eprint!("The ordered recipients: {:?}", recipients);
        recipients
    }
}

impl ContractInterface for Contract {
    fn construct(mixer_eth_addr: H160) {
        let mixer_eth_addr_str: String = mixer_eth_addr.to_hex();
        write_state!(MIXER_ETH_ADDR => mixer_eth_addr_str);

        // Create new random encryption key
        let key = generate_key();
        write_state!(ENCRYPTION_KEY => key);
    }

    fn get_pub_key() -> Vec<u8> {
        let keypair = Self::get_keypair();
        let pub_key = keypair.get_pubkey();
        let pub_key_text: String = pub_key.to_hex();
        eprint!("The pubKey hex: {}", pub_key_text);
        pub_key.to_vec()
    }

    fn execute_deal(
        operator_address: H160,
        operator_nonce: U256, // TODO: Try with lower integer
        nb_recipients: U256,
        amount: U256,
        pub_keys: Vec<Vec<u8>>,
        enc_recipients: Vec<Vec<u8>>,
        senders: Vec<H160>,
        signatures: Vec<Vec<u8>>,
    ) -> Vec<H160> {
        eprint!(
            "In execute_deal({:?}, {:?}, {:?}, {:?}, {:?})",
            operator_address, operator_nonce, nb_recipients, pub_keys, enc_recipients
        );
        let mut recipients = Self::verify_deposits_internal(
            nb_recipients,
            amount,
            pub_keys,
            enc_recipients,
            senders.clone(),
            signatures);
        // TODO: Use the rand service
        let seed = 10;
        for i in (0..recipients.len()).rev() {
            let j = seed % (i + 1);
            let recipient = recipients[j];
            recipients[j] = recipients[i];
            recipients[i] = recipient;
        }
        eprint!("The mixed recipients: {:?}", recipients);
        let mixer_eth_addr: String = Self::get_mixer_eth_addr();
        let prefixed_eth_addr = format!("0x{}", mixer_eth_addr);
        eprint!("The smart contract address: {}", prefixed_eth_addr);
        let eth_contract = EthContract::new(&prefixed_eth_addr);
        let deal_id = Self::generate_deal_id(&amount,
                                             &senders,
                                             &operator_address,
                                             &operator_nonce);
        eprint!("The DealId: {:?}", deal_id);
        // TODO: Converting as a workaround for lack of bytes32 support
        let deal_id_uint = U256::from(deal_id);
        eth_contract.distribute(deal_id_uint, recipients.clone());
        return recipients;
    }

    fn verify_deposits(
        nb_recipients: U256,
        amount: U256,
        pub_keys: Vec<Vec<u8>>,
        enc_recipients: Vec<Vec<u8>>,
        senders: Vec<H160>,
        signatures: Vec<Vec<u8>>,
    ) -> bool {
        Self::verify_deposits_internal(nb_recipients, amount, pub_keys, enc_recipients, senders, signatures);
        true
    }
}

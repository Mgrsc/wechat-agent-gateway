use aes::Aes128;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, block_padding::Pkcs7};
use base64::Engine;
use ecb::{Decryptor, Encryptor};
use rand::Rng;

use crate::errors::AppError;
use crate::models::{EventKind, MediaDescriptor, MediaRawRef};

type Aes128EcbEnc = Encryptor<Aes128>;
type Aes128EcbDec = Decryptor<Aes128>;

#[derive(Debug, Clone)]
pub struct StoredMediaAsset {
    pub account_id: String,
    pub descriptor: MediaDescriptor,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct UploadedMediaRef {
    pub download_encrypted_query_param: String,
    pub aes_key_hex: String,
    pub aes_key_base64: String,
    pub plaintext_size: usize,
    pub ciphertext_size: usize,
}

pub fn build_outbound_media_asset(
    account_id: String,
    kind: EventKind,
    filename: Option<String>,
    mime: Option<String>,
    bytes: Vec<u8>,
) -> Result<StoredMediaAsset, AppError> {
    if bytes.is_empty() {
        return Err(AppError::InvalidMediaUpload("file is empty".into()));
    }

    let media_id = format!("med_out_{}", random_hex_16());
    let checksum_md5 = Some(format!("{:x}", md5::compute(&bytes)));
    let descriptor = MediaDescriptor {
        media_id,
        kind,
        filename,
        mime,
        size: Some(bytes.len() as u64),
        width: None,
        height: None,
        duration_ms: None,
        checksum_md5,
        transcript: None,
        raw_ref: MediaRawRef::default(),
    };

    Ok(StoredMediaAsset {
        account_id,
        descriptor,
        bytes,
    })
}

pub fn random_hex_16() -> String {
    let bytes: [u8; 16] = rand::random();
    hex_encode(&bytes)
}

pub fn generate_aes_key() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::rng().fill(&mut bytes);
    bytes
}

pub fn encrypt_aes_ecb_pkcs7(plaintext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, AppError> {
    let cipher = Aes128EcbEnc::new_from_slice(key)
        .map_err(|error| AppError::WechatApi(error.to_string()))?;
    let block_size = 16usize;
    let mut buf = plaintext.to_vec();
    let pos = buf.len();
    let padding = block_size - (pos % block_size);
    buf.resize(pos + padding, 0);
    let ciphertext = cipher
        .encrypt_padded_mut::<Pkcs7>(&mut buf, pos)
        .map_err(|error| AppError::WechatApi(error.to_string()))?;
    Ok(ciphertext.to_vec())
}

pub fn decrypt_aes_ecb_pkcs7(ciphertext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, AppError> {
    let cipher = Aes128EcbDec::new_from_slice(key)
        .map_err(|error| AppError::WechatApi(error.to_string()))?;
    let mut buf = ciphertext.to_vec();
    let plaintext = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|error| AppError::WechatApi(error.to_string()))?;
    Ok(plaintext.to_vec())
}

pub fn aes_ecb_padded_size(plaintext_size: usize) -> usize {
    ((plaintext_size / 16) + 1) * 16
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(nibble_to_hex(byte >> 4));
        out.push(nibble_to_hex(byte & 0x0f));
    }
    out
}

pub fn base64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn base64_decode(value: &str) -> Result<Vec<u8>, AppError> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| AppError::WechatApi(error.to_string()))
}

pub fn parse_aes_key_base64(value: &str) -> Result<[u8; 16], AppError> {
    let decoded = base64_decode(value)?;
    if decoded.len() == 16 {
        let mut out = [0u8; 16];
        out.copy_from_slice(&decoded);
        return Ok(out);
    }

    if decoded.len() == 32 && decoded.iter().all(u8::is_ascii_hexdigit) {
        let mut out = [0u8; 16];
        for (index, chunk) in decoded.chunks_exact(2).enumerate() {
            let value = std::str::from_utf8(chunk)
                .map_err(|error| AppError::WechatApi(error.to_string()))?;
            out[index] = u8::from_str_radix(value, 16)
                .map_err(|error| AppError::WechatApi(error.to_string()))?;
        }
        return Ok(out);
    }

    Err(AppError::WechatApi(format!(
        "invalid aes_key encoding length {}",
        decoded.len()
    )))
}

fn nibble_to_hex(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + (nibble - 10)) as char,
        _ => unreachable!("nibble out of range"),
    }
}

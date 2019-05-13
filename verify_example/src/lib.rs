//! This code is mostly copied from:
//! https://docs.sequoia-pgp.org/sequoia_guide/chapter_01/index.html

use failure::{err_msg, Error};
use sequoia_openpgp::parse::stream::{
    MessageLayer, MessageStructure, VerificationHelper, VerificationResult, Verifier,
};
use sequoia_openpgp::parse::{PacketParser, Parse};
use sequoia_openpgp::{KeyID, TPK};
use serde::Deserialize;
use std::io;

pub static JACK_PGP_KEY: &str = include_str!("jack_key.pgp");
pub static ROOT_PGP_KEY: &str = include_str!("root_key.pgp");
pub static ROOT_5360668_PGP: &str = include_str!("root_5360668.pgp");
pub static ROOT_5360668_KBSIG: &str = include_str!("root_5360668.kbsig");
pub static ROOT_5360668_JSON: &str = include_str!("root_5360668.json");
pub static ROOT_NACL_KEY: &str =
    "01209ec31411b9b287f62630c2486005af27548ba62a59bbc802e656b888991a20230a";

const BAD_SIG: &str = "signature verification failed";
const MISSING_KEY: &str = "missing PGP key for signature";

pub fn load_pgp_key(key: &[u8]) -> Result<TPK, Error> {
    let parser_result = PacketParser::from_bytes(key)?;
    TPK::from_packet_parser(parser_result)
}

/// Verifies the given message.
pub fn verify_pgp(signed_message: &[u8], sender: &TPK) -> Result<Vec<u8>, Error> {
    // Make a helper that that feeds the sender's public key to the
    // verifier.
    let helper = Helper { tpk: sender };

    // Now, create a verifier with a helper using the given TPKs.
    let mut verifier = Verifier::from_bytes(signed_message, helper, None)?;

    // Verify the data.
    let mut output = Vec::new();
    io::copy(&mut verifier, &mut output)?;
    Ok(output)
}

struct Helper<'a> {
    tpk: &'a TPK,
}

impl<'a> VerificationHelper for Helper<'a> {
    fn get_public_keys(&mut self, _ids: &[KeyID]) -> Result<Vec<TPK>, Error> {
        // Return public keys for signature verification here.
        Ok(vec![self.tpk.clone()])
    }

    fn check(&mut self, structure: &MessageStructure) -> Result<(), Error> {
        // In this function, we implement our signature verification
        // policy.

        let mut good = false;
        for (i, layer) in structure.iter().enumerate() {
            match (i, layer) {
                // First, we are interested in signatures over the
                // data, i.e. level 0 signatures.
                (0, MessageLayer::SignatureGroup { ref results }) => {
                    // Finally, given a VerificationResult, which only says
                    // whether the signature checks out mathematically, we apply
                    // our policy.
                    match results.get(0) {
                        Some(VerificationResult::GoodChecksum(..)) => good = true,
                        Some(VerificationResult::MissingKey(_)) => {
                            return Err(err_msg(MISSING_KEY))
                        }
                        Some(VerificationResult::BadChecksum(_)) => return Err(err_msg(BAD_SIG)),
                        None => return Err(err_msg("No signature")),
                    }
                }
                _ => return Err(err_msg("Unexpected message structure")),
            }
        }

        if good {
            Ok(()) // Good signature.
        } else {
            Err(err_msg("Signature verification failed"))
        }
    }
}

pub fn load_nacl_key(kid: &str) -> Result<sodiumoxide::crypto::sign::PublicKey, Error> {
    let bytes = hex::decode(&kid)?;
    // Strip the prefix and suffix.
    let type_bytes = &bytes[..2];
    let suffix_bytes = &bytes[bytes.len() - 1..];
    let key_bytes = &bytes[2..bytes.len() - 1];
    if type_bytes != &[0x01, 0x20] {
        return Err(err_msg(format!("wrong key type: {:?}", type_bytes)));
    }
    if suffix_bytes != &[0x0a] {
        return Err(err_msg(format!("wrong key suffix: {:?}", suffix_bytes)));
    }
    sodiumoxide::crypto::sign::PublicKey::from_slice(key_bytes).ok_or(err_msg("bad key length"))
}

#[derive(Debug, Deserialize)]
struct KBSig {
    body: KBSigBody,
}

#[derive(Debug, Deserialize)]
struct KBSigBody {
    // Without these bytes annotations, serde will assume Vec<u8> is an array
    // of its, rather than the msgpack bytes type.
    #[serde(with = "serde_bytes")]
    sig: Vec<u8>,
    #[serde(with = "serde_bytes")]
    payload: Vec<u8>,
}

pub fn verify_kbsig(
    signed_message: &str,
    key: &sodiumoxide::crypto::sign::PublicKey,
) -> Result<Vec<u8>, Error> {
    let bytes = base64::decode(signed_message)?;
    // let v: rmpv::Value = rmpv::decode::read_value(&mut &*bytes).unwrap();
    // dbg!(v);
    let obj: KBSig = rmp_serde::decode::from_slice(&bytes)?;
    let sig = sodiumoxide::crypto::sign::Signature::from_slice(&obj.body.sig)
        .ok_or(err_msg("bad sig length"))?;
    if !sodiumoxide::crypto::sign::verify_detached(&sig, &obj.body.payload, key) {
        return Err(err_msg(BAD_SIG));
    }
    Ok(obj.body.payload)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_pgp_verify_merkle_root() {
        let key = load_pgp_key(ROOT_PGP_KEY.as_bytes()).unwrap();
        let output = verify_pgp(ROOT_5360668_PGP.as_bytes(), &key).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(ROOT_5360668_JSON, output_str);
    }

    #[test]
    fn test_pgp_missing_key_fails() {
        let key = load_pgp_key(JACK_PGP_KEY.as_bytes()).unwrap();
        let err = verify_pgp(ROOT_5360668_PGP.as_bytes(), &key).unwrap_err();
        assert_eq!(err.to_string(), MISSING_KEY);
    }

    // TODO: Produce a corrupt signature with a valid CRC and check that.

    #[test]
    fn test_nacl_verify_merkle_root() {
        let key = load_nacl_key(ROOT_NACL_KEY).unwrap();
        let output = verify_kbsig(ROOT_5360668_KBSIG, &key).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(ROOT_5360668_JSON, output_str);
    }

    #[test]
    fn test_nacl_verify_wrong_key_fails() {
        let key = sodiumoxide::crypto::sign::PublicKey([0xff; 32]);
        let err = verify_kbsig(ROOT_5360668_KBSIG, &key).unwrap_err();
        assert_eq!(err.to_string(), BAD_SIG);
    }
}

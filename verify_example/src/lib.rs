//! This code is mostly copied from:
//! https://docs.sequoia-pgp.org/sequoia_guide/chapter_01/index.html

use sequoia_openpgp::parse::stream::{
    MessageLayer, MessageStructure, VerificationHelper, VerificationResult, Verifier,
};
use sequoia_openpgp::parse::{PacketParser, Parse};
use sequoia_openpgp::{KeyID, TPK};
use std::io;

pub fn load_key(key: &[u8]) -> sequoia_openpgp::Result<TPK> {
    let parser_result = PacketParser::from_bytes(key)?;
    TPK::from_packet_parser(parser_result)
}

/// Verifies the given message.
pub fn verify(signed_message: &[u8], sender: &TPK) -> sequoia_openpgp::Result<Vec<u8>> {
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
    fn get_public_keys(&mut self, _ids: &[KeyID]) -> sequoia_openpgp::Result<Vec<TPK>> {
        // Return public keys for signature verification here.
        Ok(vec![self.tpk.clone()])
    }

    fn check(&mut self, structure: &MessageStructure) -> sequoia_openpgp::Result<()> {
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
                            return Err(failure::err_msg("Missing key to verify signature"))
                        }
                        Some(VerificationResult::BadChecksum(_)) => {
                            return Err(failure::err_msg("Bad signature"))
                        }
                        None => return Err(failure::err_msg("No signature")),
                    }
                }
                _ => return Err(failure::err_msg("Unexpected message structure")),
            }
        }

        if good {
            Ok(()) // Good signature.
        } else {
            Err(failure::err_msg("Signature verification failed"))
        }
    }
}

#[test]
fn verify_merkle_root() -> sequoia_openpgp::Result<()> {
    static ROOT_KEY: &str = include_str!("root_key.pgp");
    static ROOT_5360668: &str = include_str!("root_5360668.pgp");
    let key = load_key(ROOT_KEY.as_bytes())?;
    let output = verify(ROOT_5360668.as_bytes(), &key)?;
    println!("verified message:\n\n{}", String::from_utf8_lossy(&output));
    Ok(())
}

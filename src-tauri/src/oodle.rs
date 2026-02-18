//! Oodle decompression for Palworld `.sav` files (PLM format, save_type 0x31).
//!
//! Uses the open-source `oozextract` crate — a pure Rust implementation of
//! Kraken / Mermaid / Selkie / Leviathan decompressors.  No external DLL
//! or proprietary library is required.

/// Decompress an Oodle-compressed buffer.
///
/// * `compressed`       – raw compressed bytes (payload after the SAV header).
/// * `uncompressed_len` – expected output size (from the SAV header).
pub fn decompress(compressed: &[u8], uncompressed_len: usize) -> Result<Vec<u8>, String> {
    let mut output = vec![0u8; uncompressed_len];
    let mut extractor = oozextract::Extractor::new();
    extractor.read_from_slice(compressed, &mut output)
        .map_err(|e| format!("Oodle decompress failed: {e:?}"))?;

    // Validate the decompressed data starts with GVAS magic (0x47 0x56 0x41 0x53)
    if output.len() >= 4 && &output[..4] != b"GVAS" {
        return Err(format!(
            "Oodle decompressed data does not start with GVAS magic (got {:02X}{:02X}{:02X}{:02X})",
            output[0], output[1], output[2], output[3]
        ));
    }
    Ok(output)
}

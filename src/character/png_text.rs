use std::fmt;

use crc32fast::Hasher;

pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

#[derive(Debug, PartialEq, Eq)]
pub enum PngTextError {
    InvalidSignature,
    TruncatedChunk,
    InvalidChunkLength,
    InvalidCrc { chunk_type: [u8; 4] },
    MalformedText(&'static str),
    MissingKeyword(String),
}

impl fmt::Display for PngTextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngTextError::InvalidSignature => write!(f, "file is not a PNG"),
            PngTextError::TruncatedChunk => write!(f, "unexpected end of PNG data"),
            PngTextError::InvalidChunkLength => {
                write!(f, "chunk length exceeds PNG bounds")
            }
            PngTextError::InvalidCrc { chunk_type } => {
                write!(
                    f,
                    "chunk {} failed CRC validation",
                    display_chunk_type(chunk_type)
                )
            }
            PngTextError::MalformedText(reason) => {
                write!(f, "malformed tEXt chunk: {}", reason)
            }
            PngTextError::MissingKeyword(keyword) => {
                write!(f, "missing '{}' tEXt metadata", keyword)
            }
        }
    }
}

impl std::error::Error for PngTextError {}

pub fn extract_text(data: &[u8], keyword: &str) -> Result<String, PngTextError> {
    if data.len() < PNG_SIGNATURE.len() || data[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return Err(PngTextError::InvalidSignature);
    }

    let mut offset = PNG_SIGNATURE.len();
    while offset + 12 <= data.len() {
        let length = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
        let chunk_type: [u8; 4] = data[offset + 4..offset + 8]
            .try_into()
            .expect("slice of length 4");
        let data_start = offset + 8;
        let data_end = data_start
            .checked_add(length)
            .ok_or(PngTextError::InvalidChunkLength)?;
        if data_end > data.len() {
            return Err(PngTextError::TruncatedChunk);
        }
        if data_end + 4 > data.len() {
            return Err(PngTextError::TruncatedChunk);
        }
        let chunk_data = &data[data_start..data_end];
        let crc_bytes: [u8; 4] = data[data_end..data_end + 4]
            .try_into()
            .expect("slice of length 4");
        let actual_crc = u32::from_be_bytes(crc_bytes);
        let mut hasher = Hasher::new();
        hasher.update(&chunk_type);
        hasher.update(chunk_data);
        let expected_crc = hasher.finalize();
        if actual_crc != expected_crc {
            return Err(PngTextError::InvalidCrc { chunk_type });
        }

        if &chunk_type == b"tEXt" {
            let Some(null_pos) = chunk_data.iter().position(|&b| b == 0) else {
                return Err(PngTextError::MalformedText("missing keyword separator"));
            };
            let keyword_bytes = &chunk_data[..null_pos];
            let value_bytes = &chunk_data[null_pos + 1..];
            let chunk_keyword: String = keyword_bytes.iter().map(|&b| b as char).collect();
            if chunk_keyword == keyword {
                let text: String = value_bytes.iter().map(|&b| b as char).collect();
                return Ok(text);
            }
        }

        offset = data_end + 4;
        if &chunk_type == b"IEND" {
            break;
        }
    }

    Err(PngTextError::MissingKeyword(keyword.to_string()))
}

fn display_chunk_type(chunk_type: &[u8; 4]) -> String {
    chunk_type
        .iter()
        .map(|&b| {
            if (32..=126).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_signature() {
        let result = extract_text(b"notpng", "chara");
        assert!(matches!(result, Err(PngTextError::InvalidSignature)));
    }

    #[test]
    fn extracts_requested_text() {
        let png = build_png(Some(b"value"), true);
        let text = extract_text(&png, "chara").unwrap();
        assert_eq!(text, "value");
    }

    #[test]
    fn reports_missing_keyword() {
        let png = build_png(None, true);
        let result = extract_text(&png, "chara");
        assert!(matches!(result, Err(PngTextError::MissingKeyword(_))));
    }

    #[test]
    fn rejects_invalid_crc() {
        let png = build_png(Some(b"value"), false);
        let result = extract_text(&png, "chara");
        assert!(matches!(result, Err(PngTextError::InvalidCrc { .. })));
    }

    const TEST_IHDR: [u8; 13] = [
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00,
    ];

    const TEST_IDAT: [u8; 12] = [
        0x78, 0xDA, 0x63, 0x60, 0x60, 0x60, 0x00, 0x00, 0x00, 0x04, 0x00, 0x01,
    ];

    fn build_png(chara_payload: Option<&[u8]>, valid_crc: bool) -> Vec<u8> {
        let mut png = Vec::new();
        png.extend_from_slice(&PNG_SIGNATURE);
        png.extend_from_slice(&chunk(*b"IHDR", &TEST_IHDR, true));
        if let Some(payload) = chara_payload {
            let mut text_data = Vec::new();
            text_data.extend_from_slice(b"chara");
            text_data.push(0);
            text_data.extend_from_slice(payload);
            png.extend_from_slice(&chunk(*b"tEXt", &text_data, valid_crc));
        }
        png.extend_from_slice(&chunk(*b"IDAT", &TEST_IDAT, true));
        png.extend_from_slice(&chunk(*b"IEND", &[], true));
        png
    }

    fn chunk(chunk_type: [u8; 4], data: &[u8], valid_crc: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + data.len());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(&chunk_type);
        out.extend_from_slice(data);
        let mut hasher = Hasher::new();
        hasher.update(&chunk_type);
        hasher.update(data);
        let mut crc = hasher.finalize();
        if !valid_crc {
            crc ^= 0xFFFF_FFFF;
        }
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }
}

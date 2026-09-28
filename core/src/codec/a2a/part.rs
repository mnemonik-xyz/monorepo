//! A2A Part type -- Rust mirror of the A2A v1.0.0-rc Part union wire object.
//!
//! A Part is one of: TextPart, FilePart, or DataPart. The `kind` discriminant
//! field maps directly from the A2A spec field names.

use serde::{Deserialize, Serialize};

/// A2A Part (A2A v1.0.0-rc) -- one element in a Message or Artifact's `parts` array.
///
/// Variants match the A2A spec `kind` values: "text", "file", "data".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Part {
    /// A plain-text part.
    Text { text: String },
    /// A file reference part (uri or inline bytes as base64).
    File {
        #[serde(skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
        #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        /// Inline file bytes encoded as base64 (optional).
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<String>,
    },
    /// A structured data part (arbitrary JSON payload).
    Data {
        data: serde_json::Value,
        #[serde(rename = "mimeType", skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_part_roundtrip() {
        let p = Part::Text {
            text: "hello".to_string(),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "text");
        assert_eq!(json["text"], "hello");
        let back: Part = serde_json::from_value(json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn file_part_roundtrip() {
        let p = Part::File {
            uri: Some("https://example.com/file.pdf".to_string()),
            mime_type: Some("application/pdf".to_string()),
            data: None,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "file");
        let back: Part = serde_json::from_value(json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn data_part_roundtrip() {
        let p = Part::Data {
            data: serde_json::json!({"key": "value"}),
            mime_type: None,
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "data");
        let back: Part = serde_json::from_value(json).unwrap();
        assert_eq!(back, p);
    }
}

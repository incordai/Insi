//! How far a piece of outside text is believed.
//!
//! Every item a connector returns carries one. Only what the repository's own
//! people wrote and reviewed is `Trusted`; what the repository's own
//! machinery produced (CI output) is `Internal`; anything anyone can write
//! (an email, an outside issue, an error report quoting user input, a web
//! page) is `Untrusted`, and is shown to an agent as data, never as
//! something to do.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    Trusted,
    Internal,
    Untrusted,
}

impl Trust {
    pub fn as_str(self) -> &'static str {
        match self {
            Trust::Trusted => "trusted",
            Trust::Internal => "internal",
            Trust::Untrusted => "untrusted",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_reads_and_writes_as_a_word() {
        assert_eq!(
            serde_json::to_string(&Trust::Untrusted).expect("json"),
            "\"untrusted\""
        );
        let t: Trust = serde_json::from_str("\"internal\"").expect("parse");
        assert_eq!(t, Trust::Internal);
        assert!(serde_json::from_str::<Trust>("\"maybe\"").is_err());
        assert!(
            Trust::Trusted < Trust::Untrusted,
            "ordered most to least believed"
        );
    }
}

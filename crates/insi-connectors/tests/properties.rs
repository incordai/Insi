use insi_core::{config::CommandConnector, topic::Topics, Trust};
use proptest::prelude::*;

proptest! {
    #[test]
    fn untrusted_connector_json_never_panics(text in ".{0,4096}") {
        let config = CommandConnector { name: "fixture".into(), command: vec![], trust: Trust::Untrusted };
        let topics = Topics { base: "fixture", rules: &[] };
        let _ = insi_connectors::command::parse(&text, &config, &topics);
    }

    #[test]
    fn connector_payload_retains_text_and_cannot_escape_topic(title in ".{0,128}", body in ".{0,512}") {
        let config = CommandConnector { name: "fixture".into(), command: vec![], trust: Trust::Untrusted };
        let topics = Topics { base: "fixture", rules: &[] };
        let payload = serde_json::json!({"title":title,"body":body,"topic":"elsewhere"}).to_string();
        let parsed = insi_connectors::command::parse(&payload, &config, &topics);
        prop_assert!(parsed.is_ok());
        if let Ok(items) = parsed {
            prop_assert_eq!(items.len(), 1);
            prop_assert_eq!(&items[0].title, &title);
            prop_assert_eq!(&items[0].body, &body);
            prop_assert_eq!(&items[0].topic, "fixture");
            prop_assert_eq!(items[0].trust, Trust::Untrusted);
        }
    }
}

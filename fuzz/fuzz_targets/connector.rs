#![no_main]
libfuzzer_sys::fuzz_target!(|text: &str| {
    let config = insi_core::config::CommandConnector {
        name: "fuzz".into(),
        command: vec![],
        trust: insi_core::Trust::Untrusted,
    };
    let topics = insi_core::topic::Topics {
        base: "fuzz",
        rules: &[],
    };
    let _ = insi_connectors::command::parse(text, &config, &topics);
});

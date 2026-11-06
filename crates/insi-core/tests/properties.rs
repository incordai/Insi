#![allow(
    clippy::expect_used,
    reason = "property failures must retain the shrinking input"
)]
use insi_core::{Config, Task};
use proptest::prelude::*;

proptest! {
    #[test]
    fn config_round_trips_arbitrary_arguments(arg in ".{0,512}") {
        let config = Config { test_cmd: vec!["tool".into(), arg], ..Config::default() };
        let encoded = config.to_toml().expect("serialize");
        prop_assert_eq!(Config::parse(&encoded).expect("parse"), config);
    }

    #[test]
    fn tasks_round_trip_and_reject_traversal(stem in "[a-z][a-z0-9_]{0,32}", prompt in ".{0,512}") {
        let mut task: Task = serde_json::from_value(serde_json::json!({
            "id": &stem, "source": "fixture", "base": "base", "reference": "reference",
            "prompt": prompt, "hidden_tests": [format!("tests/{stem}.rs")],
            "check": ["runner", "{tests}"], "budget": {"wall_secs": 10}
        })).expect("fixture");
        prop_assert!(task.validate().is_ok());
        let decoded: Task = toml::from_str(&toml::to_string(&task).expect("serialize")).expect("parse");
        prop_assert_eq!(&decoded, &task);
        prop_assert!(decoded.validate().is_ok());
        task.hidden_tests = vec![format!("../{stem}.rs")];
        prop_assert!(task.validate().is_err());
    }

    #[test]
    fn untrusted_toml_never_panics(text in ".{0,4096}") {
        let _ = Config::parse(&text);
        if let Ok(task) = toml::from_str::<Task>(&text) {
            let _ = task.validate();
            let _ = task.check_argv();
        }
    }
}

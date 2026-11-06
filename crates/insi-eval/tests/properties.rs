use insi_core::EvalResult;
use proptest::prelude::*;

proptest! {
    #[test]
    fn worker_reports_round_trip_usage(input in 0u64..1_000_000, output in 0u64..1_000_000, model in "[a-z]{1,24}") {
        let encoded = serde_json::json!({"type":"result", "model":model, "usage":{"input_tokens":input,"output_tokens":output}}).to_string();
        let reported = insi_eval::run::reported(&encoded);
        prop_assert!(reported.is_some());
        if let Some((_, usage)) = reported {
            prop_assert_eq!(usage.input, input);
            prop_assert_eq!(usage.output, output);
        }
    }

    #[test]
    fn untrusted_json_and_worker_output_never_panic(text in ".{0,4096}") {
        let _ = insi_eval::run::reported(&text);
        if let Ok(result) = serde_json::from_str::<EvalResult>(&text) {
            let encoded = serde_json::to_string(&result);
            prop_assert!(encoded.is_ok());
        }
    }
}

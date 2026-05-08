use regex::Regex;
use stim_sidecar::stdout::{extract_endpoint, strip_ansi};

mod strip {
    use super::*;

    #[test]
    fn removes_colors() {
        let raw = "\x1b[1m\x1b[92m  Local:\x1b[0m   http://127.0.0.1:1421/";
        assert_eq!(strip_ansi(raw), "  Local:   http://127.0.0.1:1421/");
    }

    #[test]
    fn preserves_plain_text() {
        let raw = "Local: http://127.0.0.1:1234/";
        assert_eq!(strip_ansi(raw), "Local: http://127.0.0.1:1234/");
    }
}

mod endpoint {
    use super::*;

    fn local_pattern() -> Regex {
        Regex::new(r"Local:\s+(http://[^\s]+)").unwrap()
    }

    #[test]
    fn returns_first_capture() {
        let pattern = local_pattern();
        let line = "  Local:   http://127.0.0.1:53321/";
        assert_eq!(
            extract_endpoint(line, &pattern).as_deref(),
            Some("http://127.0.0.1:53321/")
        );
    }

    #[test]
    fn strips_ansi_first() {
        let pattern = local_pattern();
        let line = "\x1b[1m\x1b[92m  Local:\x1b[0m   http://127.0.0.1:1421/";
        assert_eq!(
            extract_endpoint(line, &pattern).as_deref(),
            Some("http://127.0.0.1:1421/")
        );
    }

    #[test]
    fn returns_none_on_miss() {
        let pattern = local_pattern();
        assert_eq!(extract_endpoint("warning: nothing here", &pattern), None);
    }

    #[test]
    fn requires_capture_group() {
        let pattern = Regex::new(r"Local:\s+http://[^\s]+").unwrap();
        assert_eq!(
            extract_endpoint("Local: http://127.0.0.1:1234/", &pattern),
            None
        );
    }
}

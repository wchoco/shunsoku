//! shunsoku のロジック層。`windows` クレートに依存せず、Windows API なしでテストできる（ADR-0005）。

/// アプリの名前。
pub const APP_NAME: &str = "shunsoku";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_name_is_shunsoku() {
        assert_eq!(APP_NAME, "shunsoku");
    }
}

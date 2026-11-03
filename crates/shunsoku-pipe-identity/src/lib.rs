//! 名前付きパイプの相手を確かめるための、ユーザー（SID）・セッション・整合性レベルの取得。
//!
//! 常駐本体（`pipe_server`）と `shunsoku-cli` の両方が使う。`shunsoku-core` は `windows` クレートに
//! 依存させない（ADR-0005）ので、別のクレートに置く。取得は Windows でだけ中身がある。

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::{
    current_integrity, current_session, current_user_sid, process_integrity, process_user_sid,
};

/// トークンの整合性レベル（mandatory label の SID `S-1-16-<RID>` の RID）。大きいほど高い。
///
/// 同じユーザーでも、昇格した側と昇格していない側は SID とセッションが同じになる。そのため常駐本体は、
/// 自分より整合性レベルの低い相手をパイプで受けない（受けると、低い側が本体の権限でファイルを書かせられる）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Integrity(pub u32);

impl Integrity {
    pub const UNTRUSTED: Integrity = Integrity(0x0000);
    pub const LOW: Integrity = Integrity(0x1000);
    pub const MEDIUM: Integrity = Integrity(0x2000);
    pub const HIGH: Integrity = Integrity(0x3000);
    pub const SYSTEM: Integrity = Integrity(0x4000);

    /// この整合性レベルで動く常駐本体が、`client` の整合性レベルの相手を受けてよいか（同じか高いなら受ける）。
    pub fn admits(self, client: Integrity) -> bool {
        client >= self
    }

    /// この整合性レベルで動く CLI が、`server` の整合性レベルで動くパイプの相手を常駐本体として信じてよいか。
    ///
    /// 低い側にパイプの名前を先に取られていたら、なりすまされる（応答を好きに返され、前面にする権利を渡す）。
    /// 昇格した CLI が昇格していない常駐本体に頼むのは正しい使い方なので、求めるのは「自分か Medium の低いほう」以上。
    pub fn trusts_server(self, server: Integrity) -> bool {
        server >= self.min(Integrity::MEDIUM)
    }

    /// パイプのセキュリティ記述子に足す SACL（SDDL）。この整合性レベルより低い相手には、読み書きで開けさせない。
    pub fn mandatory_label(self) -> String {
        format!("S:(ML;;NWNR;;;S-1-16-{})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resident_admits_clients_at_its_own_level_or_higher() {
        assert!(Integrity::MEDIUM.admits(Integrity::MEDIUM));
        assert!(Integrity::MEDIUM.admits(Integrity::HIGH));
        assert!(Integrity::HIGH.admits(Integrity::HIGH));
        assert!(Integrity::HIGH.admits(Integrity::SYSTEM));
    }

    #[test]
    fn a_resident_refuses_clients_below_its_level() {
        assert!(!Integrity::HIGH.admits(Integrity::MEDIUM));
        assert!(!Integrity::HIGH.admits(Integrity(0x2100))); // medium plus
        assert!(!Integrity::MEDIUM.admits(Integrity::LOW));
        assert!(!Integrity::MEDIUM.admits(Integrity::UNTRUSTED));
    }

    #[test]
    fn a_cli_trusts_a_server_at_its_own_level_or_medium() {
        assert!(Integrity::MEDIUM.trusts_server(Integrity::MEDIUM));
        assert!(Integrity::MEDIUM.trusts_server(Integrity::HIGH));
        assert!(Integrity::HIGH.trusts_server(Integrity::MEDIUM)); // 昇格した CLI から常駐本体へ
        assert!(Integrity::LOW.trusts_server(Integrity::LOW));
    }

    #[test]
    fn a_cli_refuses_a_server_below_its_level_and_medium() {
        assert!(!Integrity::MEDIUM.trusts_server(Integrity::LOW));
        assert!(!Integrity::HIGH.trusts_server(Integrity::LOW));
        assert!(!Integrity::MEDIUM.trusts_server(Integrity::UNTRUSTED));
        assert!(!Integrity::LOW.trusts_server(Integrity::UNTRUSTED));
    }

    #[test]
    fn the_pipe_label_keeps_lower_levels_from_reading_or_writing() {
        assert_eq!(
            Integrity::HIGH.mandatory_label(),
            "S:(ML;;NWNR;;;S-1-16-12288)"
        );
        assert_eq!(
            Integrity::MEDIUM.mandatory_label(),
            "S:(ML;;NWNR;;;S-1-16-8192)"
        );
    }
}

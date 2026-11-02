use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_INFORMATION_CLASS,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER, TokenIntegrityLevel, TokenUser,
};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::core::{PWSTR, Result};

use crate::Integrity;

/// 閉じ忘れないためのハンドル。
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: 開いたハンドルを 1 回だけ閉じる
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// このプロセスのセッションの番号。
pub fn current_session() -> Result<u32> {
    let mut session = 0;
    // SAFETY: session はこの関数の中で生きている
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session)? };
    Ok(session)
}

/// このプロセスのユーザーの SID（`S-1-5-21-...`）。
pub fn current_user_sid() -> Result<String> {
    // SAFETY: 自分のプロセスの疑似ハンドル（閉じなくてよい）
    token_user_sid(unsafe { GetCurrentProcess() })
}

/// プロセス `pid` のユーザーの SID。
pub fn process_user_sid(pid: u32) -> Result<String> {
    // SAFETY: 開いたハンドルは Owned が閉じる
    let process = Owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)? });
    token_user_sid(process.0)
}

/// このプロセスの整合性レベル。
pub fn current_integrity() -> Result<Integrity> {
    // SAFETY: 自分のプロセスの疑似ハンドル（閉じなくてよい）
    token_integrity(unsafe { GetCurrentProcess() })
}

/// プロセス `pid` の整合性レベル。
pub fn process_integrity(pid: u32) -> Result<Integrity> {
    // SAFETY: 開いたハンドルは Owned が閉じる
    let process = Owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)? });
    token_integrity(process.0)
}

/// プロセスのトークンの `class` の情報。ポインタを含む構造体なので、u64 の並びで置き場の境界を合わせて返す。
fn token_information(process: HANDLE, class: TOKEN_INFORMATION_CLASS) -> Result<Vec<u64>> {
    let mut token = HANDLE::default();
    // SAFETY: token はこの関数の中で生きていて、Owned が閉じる
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token)? };
    let token = Owned(token);
    let mut len = 0;
    // SAFETY: 1 回目は大きさだけを聞く（失敗するのが正しい）
    let _ = unsafe { GetTokenInformation(token.0, class, None, 0, &mut len) };
    let mut buffer = vec![0u64; (len as usize).div_ceil(8).max(1)];
    // SAFETY: buffer は len バイト以上ある
    unsafe {
        GetTokenInformation(
            token.0,
            class,
            Some(buffer.as_mut_ptr().cast()),
            len,
            &mut len,
        )?;
    }
    Ok(buffer)
}

fn token_integrity(process: HANDLE) -> Result<Integrity> {
    let buffer = token_information(process, TokenIntegrityLevel)?;
    // SAFETY: GetTokenInformation が TOKEN_MANDATORY_LABEL を書いた。SID は buffer の中を指す。
    // 整合性レベルの SID（S-1-16-<RID>）の最後の副権限が RID
    unsafe {
        let label = &*(buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
        let sid = label.Label.Sid;
        let count = u32::from(*GetSidSubAuthorityCount(sid));
        if count == 0 {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_UNEXPECTED,
            ));
        }
        Ok(Integrity(*GetSidSubAuthority(sid, count - 1)))
    }
}

fn token_user_sid(process: HANDLE) -> Result<String> {
    let buffer = token_information(process, TokenUser)?;
    // SAFETY: GetTokenInformation が TOKEN_USER を書いた。SID は buffer の中を指す
    let user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut text = PWSTR::null();
    // SAFETY: SID は buffer が生きている間有効。text は LocalFree で返す
    unsafe {
        ConvertSidToStringSidW(user.User.Sid, &mut text)?;
        let sid = text.to_string();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid.map_err(|_| windows::core::Error::from(windows::Win32::Foundation::E_UNEXPECTED))
    }
}

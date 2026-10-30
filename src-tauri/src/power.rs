use crate::schedule::PowerAction;

pub fn execute(action: PowerAction) -> Result<(), String> {
    // Test instances exercise the entire countdown without powering off the PC.
    #[cfg(debug_assertions)]
    if std::env::var_os("NDM_DATA_DIR").is_some() {
        if let Some(log) = std::env::var_os("NDM_POWER_ACTION_LOG") {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(log)
                .map_err(|e| e.to_string())?;
            writeln!(file, "{action:?}").map_err(|e| e.to_string())?;
            return Ok(());
        }
        return Err("独立调试实例未配置电源动作测试日志".into());
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::{
            Foundation::*,
            Security::*,
            System::{Power::SetSuspendState, Shutdown::InitiateSystemShutdownExW, Threading::*},
        };
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return Err(format!(
                "无法获取电源操作权限：{}",
                std::io::Error::last_os_error()
            ));
        }
        let result = (|| {
            let name: Vec<u16> = "SeShutdownPrivilege\0".encode_utf16().collect();
            let mut luid = std::mem::zeroed();
            if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let mut previous: TOKEN_PRIVILEGES = std::mem::zeroed();
            let mut length = 0;
            SetLastError(0);
            if AdjustTokenPrivileges(
                token,
                0,
                &privileges,
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut previous,
                &mut length,
            ) == 0
                || GetLastError() == ERROR_NOT_ALL_ASSIGNED
            {
                return Err("Windows 未授予关机 / 睡眠权限".into());
            }
            let ok = match action {
                PowerAction::Sleep => SetSuspendState(false, false, false),
                // Never force other applications to close or discard unsaved work.
                PowerAction::Shutdown => {
                    InitiateSystemShutdownExW(
                        std::ptr::null(),
                        std::ptr::null(),
                        0,
                        0,
                        0,
                        0x80040000,
                    ) != 0
                }
            };
            let error = std::io::Error::last_os_error();
            AdjustTokenPrivileges(
                token,
                0,
                &previous,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );
            if ok {
                Ok(())
            } else {
                Err(format!("Windows 电源操作失败：{error}"))
            }
        })();
        CloseHandle(token);
        result
    }
    #[cfg(not(windows))]
    {
        let _ = action;
        Err("当前系统暂不支持电源操作".into())
    }
}

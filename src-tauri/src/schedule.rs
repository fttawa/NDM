use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadWindow {
    pub enabled: bool,
    pub start: u16,
    pub end: u16,
}

impl Default for DownloadWindow {
    fn default() -> Self {
        Self {
            enabled: false,
            start: 0,
            end: 480,
        }
    }
}

impl DownloadWindow {
    pub fn validate(&self) -> Result<(), String> {
        if self.start >= 1440 || self.end >= 1440 || self.enabled && self.start == self.end {
            return Err("下载时段须为不同的 00:00–23:59 时间；全天下载请关闭时段限制".into());
        }
        Ok(())
    }

    pub fn allows(&self, minute: u16) -> bool {
        !self.enabled
            || if self.start < self.end {
                minute >= self.start && minute < self.end
            } else {
                minute >= self.start || minute < self.end
            }
    }
}

pub fn validate_time(at: Option<u64>) -> Result<(), String> {
    if at.is_some_and(|v| v > 253_402_300_799_999) {
        return Err("计划时间超出有效范围".into());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PowerAction {
    Shutdown,
    Sleep,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompletionStatus {
    pub action: Option<PowerAction>,
    pub deadline: Option<u64>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Completion {
    pub status: CompletionStatus,
    succeeded: bool,
}

impl Completion {
    pub fn arm(&mut self, action: Option<PowerAction>) {
        *self = Self {
            status: CompletionStatus {
                action,
                ..Default::default()
            },
            succeeded: false,
        };
    }

    pub fn completed(&mut self) {
        self.succeeded = true;
    }
    pub fn interrupt(&mut self) {
        self.status.deadline = None;
    }

    /// Only real successful downloads after arming can trigger the action.
    pub fn tick(
        &mut self,
        all_completed: bool,
        idle: bool,
        updating: bool,
        now: u64,
    ) -> Option<PowerAction> {
        let action = self.status.action?;
        if !all_completed || !idle || updating || !self.succeeded {
            self.interrupt();
            return None;
        }
        let deadline = *self
            .status
            .deadline
            .get_or_insert(now.saturating_add(60_000));
        if now < deadline {
            return None;
        }
        self.arm(None);
        Some(action)
    }
}

pub fn local_minute() -> u16 {
    #[cfg(windows)]
    unsafe {
        let mut local = std::mem::zeroed();
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut local);
        local.wHour * 60 + local.wMinute
    }
    #[cfg(not(windows))]
    {
        ((crate::model::now_ms() / 60_000) % 1440) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_include_start_exclude_end_and_cross_midnight() {
        let mut window = DownloadWindow {
            enabled: true,
            start: 60,
            end: 120,
        };
        assert!(!window.allows(59));
        assert!(window.allows(60));
        assert!(window.allows(119));
        assert!(!window.allows(120));
        window.start = 1380;
        window.end = 120;
        assert!(window.allows(1380));
        assert!(window.allows(0));
        assert!(window.allows(119));
        assert!(!window.allows(120));
        assert!(!window.allows(1379));
        window.end = window.start;
        assert!(window.validate().is_err());
        window.enabled = false;
        assert!(window.allows(700));
        assert!(window.validate().is_ok());
        window.end = 1440;
        assert!(window.validate().is_err());
        assert!(validate_time(Some(u64::MAX)).is_err());
    }
    #[test]
    fn completion_waits_for_success_idle_and_countdown_and_runs_once() {
        let mut c = Completion::default();
        c.arm(Some(PowerAction::Sleep));
        assert_eq!(c.tick(true, true, false, 0), None);
        assert!(c.status.deadline.is_none());
        c.completed();
        assert_eq!(c.tick(true, false, false, 0), None);
        assert_eq!(c.tick(true, true, false, 0), None);
        assert_eq!(c.status.deadline, Some(60_000));
        assert_eq!(c.tick(true, true, false, 59_999), None);
        assert_eq!(c.tick(true, true, false, 60_000), Some(PowerAction::Sleep));
        assert_eq!(c.tick(true, true, false, 120_000), None);
    }
    #[test]
    fn new_tasks_errors_pause_updates_and_cancel_invalidate_countdown() {
        let mut c = Completion::default();
        c.arm(Some(PowerAction::Shutdown));
        c.completed();
        c.tick(true, true, false, 0);
        c.tick(false, true, false, 50_000);
        assert!(c.status.deadline.is_none());
        c.tick(true, true, false, 60_000);
        assert_eq!(c.status.deadline, Some(120_000));
        c.tick(true, true, true, 110_000);
        assert!(c.status.deadline.is_none());
        c.tick(true, true, false, 120_000);
        c.arm(None);
        assert_eq!(c.tick(true, true, false, 999_999), None);
    }
}

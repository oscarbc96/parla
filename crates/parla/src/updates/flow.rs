//! The user-consent flow of an in-app update, as a pure state machine.
//!
//! Nothing here touches the network, the disk or the UI: the manager drives
//! it and renders the result. A package is only downloaded when the user
//! clicks **Update** (`start`), and the app only restarts when the user
//! clicks **Restart to update**.

use super::download::DownloadError;
use super::install::InstallError;

/// Why an update did not reach the Ready state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateFailure {
    /// The download did not complete (network, size, disk).
    Download,
    /// The signature check refused the download.
    Signature,
    /// The new app could not be put in place.
    Install,
}

impl UpdateFailure {
    pub fn from_download(error: &DownloadError) -> Self {
        match error {
            DownloadError::Verify(_) => Self::Signature,
            _ => Self::Download,
        }
    }

    pub fn from_install(_error: &InstallError) -> Self {
        Self::Install
    }
}

/// Where the notice of an available update is in the flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePhase {
    /// The update is offered; nothing has been downloaded.
    Offer,
    Downloading {
        percent: u8,
    },
    Installing,
    /// The new app is in place; the user restarts when ready.
    Ready,
    Failed(UpdateFailure),
}

impl UpdatePhase {
    /// True when the user may click **Update** (first time or retry).
    pub fn can_start(&self) -> bool {
        matches!(self, Self::Offer | Self::Failed(_))
    }

    /// True while a download or install runs, or a finished one waits for
    /// the restart: a check result must not replace the notice then.
    pub fn holds_notice(&self) -> bool {
        matches!(
            self,
            Self::Downloading { .. } | Self::Installing | Self::Ready
        )
    }

    /// The user clicked Update. Returns false when it is not allowed now.
    pub fn start(&mut self) -> bool {
        if !self.can_start() {
            return false;
        }
        *self = Self::Downloading { percent: 0 };
        true
    }

    /// Download progress. Returns true when the shown percentage changed.
    pub fn progress(&mut self, received: u64, total: u64) -> bool {
        let Self::Downloading { percent } = self else {
            return false;
        };
        let next = received
            .min(total)
            .saturating_mul(100)
            .checked_div(total)
            .unwrap_or(0) as u8;
        if next == *percent {
            return false;
        }
        *percent = next;
        true
    }

    /// The download is verified; the install starts.
    pub fn downloaded(&mut self) -> bool {
        if !matches!(self, Self::Downloading { .. }) {
            return false;
        }
        *self = Self::Installing;
        true
    }

    /// The new app is in place.
    pub fn installed(&mut self) -> bool {
        if !matches!(self, Self::Installing) {
            return false;
        }
        *self = Self::Ready;
        true
    }

    /// A download or install step failed.
    pub fn fail(&mut self, failure: UpdateFailure) -> bool {
        if !matches!(self, Self::Downloading { .. } | Self::Installing) {
            return false;
        }
        *self = Self::Failed(failure);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::package::VerifyError;

    #[test]
    fn nothing_starts_until_the_user_clicks() {
        let phase = UpdatePhase::Offer;
        assert!(phase.can_start());
        assert!(!phase.holds_notice());
    }

    #[test]
    fn happy_path_runs_download_install_ready() {
        let mut phase = UpdatePhase::Offer;
        assert!(phase.start());
        assert_eq!(phase, UpdatePhase::Downloading { percent: 0 });
        assert!(phase.progress(50, 200));
        assert_eq!(phase, UpdatePhase::Downloading { percent: 25 });
        assert!(phase.downloaded());
        assert_eq!(phase, UpdatePhase::Installing);
        assert!(phase.installed());
        assert_eq!(phase, UpdatePhase::Ready);
    }

    #[test]
    fn progress_reports_only_a_changed_percentage() {
        let mut phase = UpdatePhase::Offer;
        phase.start();
        assert!(phase.progress(10, 1000));
        assert!(!phase.progress(11, 1000));
        assert!(!phase.progress(14, 1000));
        assert!(phase.progress(20, 1000));
        assert!(phase.progress(5000, 1000));
        assert_eq!(phase, UpdatePhase::Downloading { percent: 100 });
    }

    #[test]
    fn start_is_refused_while_busy_or_ready() {
        for mut phase in [
            UpdatePhase::Downloading { percent: 3 },
            UpdatePhase::Installing,
            UpdatePhase::Ready,
        ] {
            let before = phase;
            assert!(!phase.start());
            assert_eq!(phase, before);
            assert!(phase.holds_notice());
        }
    }

    #[test]
    fn failure_allows_a_retry() {
        let mut phase = UpdatePhase::Offer;
        phase.start();
        assert!(phase.fail(UpdateFailure::Download));
        assert_eq!(phase, UpdatePhase::Failed(UpdateFailure::Download));
        assert!(!phase.holds_notice());
        assert!(phase.start());
        assert_eq!(phase, UpdatePhase::Downloading { percent: 0 });
    }

    #[test]
    fn steps_out_of_order_are_ignored() {
        let mut phase = UpdatePhase::Offer;
        assert!(!phase.downloaded());
        assert!(!phase.installed());
        assert!(!phase.fail(UpdateFailure::Install));
        assert!(!phase.progress(1, 2));
        assert_eq!(phase, UpdatePhase::Offer);
    }

    #[test]
    fn errors_map_to_failures() {
        assert_eq!(
            UpdateFailure::from_download(&DownloadError::Verify(VerifyError::BadSignature)),
            UpdateFailure::Signature
        );
        assert_eq!(
            UpdateFailure::from_download(&DownloadError::Http),
            UpdateFailure::Download
        );
        assert_eq!(
            UpdateFailure::from_download(&DownloadError::SizeMismatch),
            UpdateFailure::Download
        );
        assert_eq!(
            UpdateFailure::from_install(&InstallError::Copy),
            UpdateFailure::Install
        );
    }
}

//! What the window needs from whatever runs behind it: the real engine or the synthetic mock.

use std::borrow::Cow;
use std::path::PathBuf;

use super::book::{Audiobook, BookAction, BookTileState, Marketplace};
use super::mock_app::{PendingLogin, Toast};
use super::preview_data::Account;
use crate::shell::browser::BrowserPage;

/// Something only the window system can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelEffect {
    Reveal(PathBuf),
}

pub trait AppModel {
    fn account(&self) -> Option<&Account>;
    fn is_signed_in(&self) -> bool {
        self.account().is_some()
    }
    fn needs_reauthentication(&self) -> bool;
    fn marketplace(&self) -> Marketplace;
    /// Also present while the device registers, so the sheet shows its spinner.
    fn pending_login(&self) -> Option<&PendingLogin>;
    fn is_loading(&self) -> bool;
    fn is_checking_marketplaces(&self) -> bool;
    fn is_library_empty(&self) -> bool;
    fn can_select(&self) -> bool;
    fn is_signed_out_and_empty(&self) -> bool {
        !self.is_signed_in() && self.is_library_empty()
    }
    /// False until saved credentials have been tried, so sign-in isn't offered too soon.
    fn has_restored(&self) -> bool {
        true
    }
    fn visible_books(&self) -> Vec<&Audiobook>;
    fn tile(&self, book: &Audiobook) -> Cow<'_, BookTileState>;
    /// Whether the book's complete audio file exists, as opposed to an unfinished download.
    fn has_download(&self, book: &Audiobook) -> bool;
    fn messages(&self) -> Vec<String>;
    fn notices(&self) -> Vec<String>;
    fn toasts(&self) -> &[Toast];
    fn selected_count(&self) -> usize;
    fn other_marketplaces(&self) -> Vec<(Marketplace, usize)>;
    /// How many selected books offer `action`.
    fn selection_offering(&self, action: BookAction) -> usize;
    fn can_sign_in(&self) -> bool;
    fn can_change_marketplace(&self) -> bool;
    fn can_sign_out(&self) -> bool;
    fn can_refresh(&self) -> bool;
    fn can_submit_pasted_url(&self) -> bool;
    /// Offers a button that stands in for finishing Amazon's page.
    fn can_simulate(&self) -> bool {
        false
    }
    /// The page for the embedded sign-in browser.
    fn browser_page(&self) -> Option<BrowserPage> {
        None
    }
    /// The sign-in page, for the user's own browser.
    fn login_url(&self) -> Option<String> {
        None
    }
    /// Whether closing the window would stop work in progress.
    fn has_active_jobs(&self) -> bool {
        false
    }
    /// Whether stopping now loses work: exports and jobs that haven't started.
    fn has_work_suspend_loses(&self) -> bool {
        false
    }
    /// Whether the work stopped by `begin_suspend` has finished stopping.
    fn has_finished_suspending(&self) -> bool {
        false
    }
    /// Whether something on screen animates by itself.
    fn is_busy(&self) -> bool;

    fn set_marketplace(&mut self, marketplace: Marketplace);
    fn sign_in(&mut self);
    /// `url` is the redirect the sign-in ended on, when there is a real one.
    fn complete_sign_in(&mut self, url: Option<String>);
    fn set_pasted_url(&mut self, url: String);
    fn submit_pasted_url(&mut self);
    fn browser_failed(&mut self, message: String);
    fn cancel_sign_in(&mut self);
    fn sign_out(&mut self);
    fn refresh(&mut self);
    fn set_search(&mut self, search: String);
    fn toggle_selection(&mut self, asin: &str);
    fn clear_selection(&mut self);
    fn perform(&mut self, action: BookAction, asin: &str) -> Option<ModelEffect>;
    fn perform_on_selection(&mut self, action: BookAction);
    /// Marks the toast copied and returns its technical details for the clipboard.
    fn copy_toast(&mut self, id: u64) -> Option<String>;
    fn dismiss_toast(&mut self, id: u64);
    fn tick(&mut self, elapsed_ms: u32);
    /// Stops every job, keeping download progress for the next launch.
    fn begin_suspend(&mut self) {}
    /// Rereads the folders, which may have changed while the app was in the background.
    fn rescan(&mut self) {}
}

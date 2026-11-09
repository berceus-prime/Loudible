//! Synthetic books, accounts and errors, ported from the Swift app's `PreviewData.swift`.
//! Nothing here refers to a real account or title.

use super::app_error::{AppError, CredentialOperation};
use super::book::{
    Asin, Audiobook, BookMenuItem, BookStatus, BookTileState, CoverColor, Marketplace,
    TransferProgress, menu_items,
};

const INDIGO: CoverColor = CoverColor(0x6E7CF7);
const TEAL: CoverColor = CoverColor(0x61CDDD);
const PINK: CoverColor = CoverColor(0xE84A62);
const GREEN: CoverColor = CoverColor(0x6BCD62);
const BROWN: CoverColor = CoverColor(0xB08C6B);
const PURPLE: CoverColor = CoverColor(0xC845E6);
const RED: CoverColor = CoverColor(0xEB5650);

/// The signed-in preview account.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub customer_name: Option<String>,
    pub marketplace: Marketplace,
}

pub fn account() -> Account {
    Account {
        customer_name: Some("Jane Doe".into()),
        marketplace: Marketplace::US,
    }
}

/// Ten books: varied titles, authors and runtimes. Books 2 and 8 have no cover, book 7 no
/// runtime. Books 3 and 8 are from the UK store and book 6 from Japan; the rest from the US.
pub fn books() -> Vec<Audiobook> {
    let book = |asin: &str,
                marketplace,
                title: &str,
                authors: &[&'static str],
                runtime,
                cover,
                size_mb: u64| {
        Audiobook {
            asin: Asin::from(asin),
            marketplace,
            title: String::from(title),
            subtitle: None,
            authors: authors.iter().map(ToString::to_string).collect(),
            narrators: vec![],
            runtime_minutes: runtime,
            cover,
            cover_url: None,
            size_bytes: size_mb * 1_000_000,
        }
    };
    vec![
        Audiobook {
            narrators: vec!["Tom Avery".into()],
            ..book(
                "PREVIEW001",
                Marketplace::US,
                "The Lighthouse Keeper's Atlas",
                &["Mara Ellison"],
                Some(642),
                Some(INDIGO),
                310,
            )
        },
        book(
            "PREVIEW002",
            Marketplace::US,
            "A Short History of Tea",
            &["Henrik Dahl"],
            Some(318),
            Some(TEAL),
            288,
        ),
        book(
            "PREVIEW003",
            Marketplace::US,
            "Signals from the Deep",
            &["Ada Okafor"],
            Some(875),
            None,
            420,
        ),
        Audiobook {
            narrators: vec!["Ana Ruiz".into()],
            ..book(
                "PREVIEW004",
                Marketplace::UK,
                "Winter in Valparaíso",
                &["Lucía Ferrer"],
                Some(501),
                Some(PINK),
                240,
            )
        },
        Audiobook {
            subtitle: Some("A Novel".into()),
            narrators: vec!["The Full Cast".into()],
            ..book(
                "PREVIEW005",
                Marketplace::US,
                "The Clockmaker's Daughter and the Brass Meridian: Being the First Book of the Long Winter Saga",
                &[
                    "Eleanor Whitcombe-Hartley",
                    "Jonathan R. Pemberton",
                    "Siobhán Ní Bhriain",
                ],
                Some(1_934),
                Some(GREEN),
                930,
            )
        },
        book(
            "PREVIEW006",
            Marketplace::US,
            "Notes on Quiet",
            &["Kenji Mori"],
            Some(196),
            Some(BROWN),
            95,
        ),
        book(
            "PREVIEW007",
            Marketplace::JAPAN,
            "Field Guide to Small Machines",
            &["Priya Raman", "Leo Brandt"],
            Some(412),
            Some(PURPLE),
            198,
        ),
        book(
            "PREVIEW008",
            Marketplace::US,
            "The Seventh Orchard",
            &["Ruth Calloway"],
            None,
            Some(RED),
            260,
        ),
        book(
            "PREVIEW009",
            Marketplace::UK,
            "Ninety Days North",
            &["Sam Okonkwo"],
            Some(733),
            None,
            350,
        ),
        book(
            "PREVIEW010",
            Marketplace::US,
            "Paper Rivers",
            &["Ines Varga"],
            Some(288),
            Some(TEAL),
            140,
        ),
    ]
}

/// Part of an unfinished download kept for later.
pub const KEPT: TransferProgress = TransferProgress {
    received: 132_000_000,
    expected: 288_000_000,
};

/// A tile with the menu that goes with its status.
pub fn tile_state(is_in_library: bool, status: BookStatus) -> BookTileState {
    BookTileState {
        is_in_library,
        menu_items: menu_items(&status, true),
        status,
        ..BookTileState::new()
    }
}

/// The plain grid tile: a badge for other stores and a Download item.
pub fn grid_tile(book: &Audiobook, is_selected: bool) -> BookTileState {
    BookTileState {
        is_selected,
        marketplace_badge: (book.marketplace != account().marketplace)
            .then(|| book.marketplace.id.to_uppercase()),
        menu_items: vec![BookMenuItem::new(super::book::BookAction::Download)],
        ..BookTileState::new()
    }
}

pub fn license_denied() -> AppError {
    AppError::Download {
        book_title: "A Short History of Tea".into(),
        technical:
            "A Short History of Tea: The download license was denied: License not granted.\n\
                    Ownership / RequesterEligibility: No purchase rights found for this title.\n\
                    Membership / CustomerThrottled: Please try again later."
                .into(),
    }
}

pub fn download_failed() -> AppError {
    AppError::Download {
        book_title: "A Short History of Tea".into(),
        technical: "HTTP 503".into(),
    }
}

pub fn export_long_title() -> AppError {
    AppError::Export {
        book_title: "The Collected Adventures of Maximiliane Alexandra Featherstonehaugh-Smythe"
            .into(),
        technical: "Conversion failed: Unable to open output file (Permission denied).".into(),
    }
}

pub fn credential_save() -> AppError {
    AppError::CredentialStorage {
        operation: CredentialOperation::Save,
        technical: "Keychain save failed: OSStatus -25293".into(),
    }
}

pub fn credential_delete() -> AppError {
    AppError::CredentialStorage {
        operation: CredentialOperation::Delete,
        technical: "Keychain delete failed: OSStatus -25293".into(),
    }
}

pub fn sign_in_browser() -> AppError {
    AppError::SignInBrowser {
        technical: "NSURLErrorDomain Code=-1009: The connection is offline.".into(),
    }
}

pub fn export_permission() -> AppError {
    AppError::Export {
        book_title: "A Short History of Tea".into(),
        technical: "Conversion failed: Could not write output file (Permission denied).".into(),
    }
}

pub const LOGIN_EXPIRED: &str = "This sign-in has expired. Start signing in again.";
pub const LOAD_FAILED: &str = "Network error: Audible returned HTTP 500 (Internal Server Error).";
pub const SESSION_EXPIRED: &str = "Your session expired. Sign in again to continue.";

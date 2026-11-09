use loudible_core::export_file_naming::*;
use loudible_core::{Audiobook, Marketplace};
use loudible_test_support::*;

fn titled(asin: &str, title: &str) -> Audiobook {
    Audiobook::new(asin, Marketplace::US, title)
}

#[test]
fn uses_title_and_asin() {
    assert_eq!(
        base_name(&titled("B0123", "Project Hail Mary"), FILE_EXTENSION),
        "Project Hail Mary [B0123]"
    );
    assert_eq!(file_name("X [B1]", 1, FILE_EXTENSION), "X [B1].m4b");
    assert_eq!(file_name("X [B1]", 3, FILE_EXTENSION), "X [B1] (3).m4b");
}

#[test]
fn sanitizes_titles() {
    for (title, expected) in [
        ("Dune: Part One", "Dune - Part One"),
        ("AC/DC \\ Live?", "AC_DC _ Live_"),
        ("A*B\"C<D>E|F", "A_B_C_D_E_F"),
        ("Line\nBreak\tTab", "Line Break Tab"),
        ("  .hidden title. ", "hidden title"),
        ("...", "Untitled"),
        ("", "Untitled"),
        ("Cafe\u{301}", "Café"),
    ] {
        assert_eq!(sanitized_title(title), expected);
    }
}

#[test]
fn sanitizes_asin() {
    assert_eq!(
        base_name(&titled("B0/../1", "T"), FILE_EXTENSION),
        "T [B01]"
    );
}

#[test]
fn windows_device_names_are_guarded() {
    assert_eq!(
        base_name(&titled("B1", "CON.notes"), FILE_EXTENSION),
        "_CON.notes [B1]"
    );
    assert_eq!(
        base_name(&titled("B1", "aux.x"), FILE_EXTENSION),
        "_aux.x [B1]"
    );
    // The ASIN tail already keeps these from being device names.
    assert_eq!(base_name(&titled("B1", "CON"), FILE_EXTENSION), "CON [B1]");
    assert_eq!(
        base_name(&titled("B1", "Console.log"), FILE_EXTENSION),
        "Console.log [B1]"
    );
}

#[test]
fn long_titles_fit_file_name_limit_with_collision_suffix() {
    let name = file_name(
        &base_name(&titled("B000000001", &"Ü".repeat(400)), FILE_EXTENSION),
        9999,
        FILE_EXTENSION,
    );
    assert!(name.len() <= 255);
    assert!(name.ends_with(" [B000000001] (9999).m4b"));
}

#[test]
fn picks_first_free_name() {
    let directory = TemporaryDirectory::default();
    for name in ["Book [B1].m4b", "Book [B1] (2).m4b"] {
        std::fs::write(directory.join(name), "").unwrap();
    }
    let path = available_path(directory.path(), "Book [B1]", FILE_EXTENSION, |p| {
        p.exists()
    });
    assert_eq!(path.file_name().unwrap(), "Book [B1] (3).m4b");
}

#[test]
fn extension_parameter() {
    assert_eq!(
        file_name(&base_name(&titled("B1", "X"), "aaxc"), 2, "aaxc"),
        "X [B1] (2).aaxc"
    );
    let long = titled("B000000001", &"Ü".repeat(400));
    assert!(file_name(&base_name(&long, "aaxc"), 9999, "aaxc").len() <= 255);
}

#[test]
fn finds_asin_in_file_name() {
    for (name, asin) in [
        ("Dune [B0123].aaxc", "B0123"),
        ("Dune [B0123] (2).aaxc", "B0123"),
        ("My copy of Dune [B0123] edited.aax", "B0123"),
        ("Dune [Book 1] [B0123].m4b", "B0123"),
        ("[B0123] [B0999].m4b", "B0999"),
    ] {
        assert_eq!(asin_in_file_name(name).as_deref(), Some(asin));
    }
}

#[test]
fn finds_no_asin() {
    for name in [
        "Dune.aaxc",
        "Dune [].aaxc",
        "Dune [B-12].aaxc",
        ".Dune [B0123].aaxc",
        ".Dune [B0123].abc.partial",
    ] {
        assert_eq!(asin_in_file_name(name), None);
    }
}

#[test]
fn partial_names_are_hidden_and_fit_the_limit() {
    let token = "00000000-0000-4000-8000-000000000001";
    let name = partial_name(&format!("{} [B1]", "Ü".repeat(200)), token);
    assert!(name.starts_with('.'));
    assert!(name.ends_with(&format!(".{token}.partial")));
    assert!(name.len() <= 255);
    assert!(is_partial(&name));
    assert!(!is_partial("Book [B1].partial"));
    assert!(!is_partial(".Book [B1].aaxc"));
}

use super::*;

fn sha(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

#[test]
fn an_original_is_sealed_stored_and_fetched_back() {
    let d = tempfile::tempdir().unwrap();
    let (sink, code) = FolderSink::create(d.path(), "card").unwrap();
    let bytes = b"the original document".to_vec();
    let h = sha(&bytes);
    assert!(!sink.has(&h));
    sink.put(&h, &bytes).unwrap();
    assert!(sink.has(&h));
    assert_eq!(sink.get(&h).unwrap(), bytes);
    // What is on the card is not the document.
    let stored = std::fs::read(d.path().join(format!("objects/{}/{h}.hqo", &h[..2]))).unwrap();
    assert!(!stored.windows(bytes.len()).any(|w| w == &bytes[..]));
    // A later session opens it with the code alone.
    let again = FolderSink::open(d.path(), &code).unwrap();
    assert_eq!(again.id(), "card");
    assert_eq!(again.get(&h).unwrap(), bytes);
    assert_eq!(again.list(), vec![h]);
}

#[test]
fn the_wrong_code_and_a_second_create_are_refused() {
    let d = tempfile::tempdir().unwrap();
    let (sink, code) = FolderSink::create(d.path(), "card").unwrap();
    let h = sha(b"x");
    sink.put(&h, b"x").unwrap();
    let other = hq_backup::object::new_recovery_code().unwrap();
    let wrong = FolderSink::open(d.path(), &other).unwrap_err();
    assert!(wrong.0.contains("does not open"), "{wrong}");
    assert!(FolderSink::open(d.path(), "hunter2").is_err());
    assert!(
        FolderSink::create(d.path(), "again").is_err(),
        "the first key must never be replaced"
    );
    assert!(FolderSink::open(d.path(), &code).is_ok());
}

#[test]
fn bytes_that_do_not_match_their_name_are_never_stored() {
    let d = tempfile::tempdir().unwrap();
    let (sink, _) = FolderSink::create(d.path(), "card").unwrap();
    assert!(sink.put(&sha(b"one"), b"two").is_err());
    assert!(sink.put("../../etc/passwd", b"x").is_err());
    assert!(sink.put("ABCD", b"x").is_err());
    assert!(sink.list().is_empty());
}

#[test]
fn a_damaged_object_is_found_by_the_drill_and_never_returned() {
    let d = tempfile::tempdir().unwrap();
    let (sink, _) = FolderSink::create(d.path(), "card").unwrap();
    let files: Vec<(String, Vec<u8>)> = (0..5u8).map(|i| (sha(&[i; 50]), vec![i; 50])).collect();
    for (h, b) in &files {
        sink.put(h, b).unwrap();
    }
    assert_eq!(
        restore_drill(&sink, 100),
        Drill {
            checked: 5,
            failed: vec![]
        }
    );
    let victim = &files[2].0;
    let path = d
        .path()
        .join(format!("objects/{}/{victim}.hqo", &victim[..2]));
    let mut raw = std::fs::read(&path).unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 1;
    std::fs::write(&path, raw).unwrap();
    assert!(sink.get(victim).is_err());
    let drill = restore_drill(&sink, 100);
    assert_eq!(drill.checked, 5);
    assert_eq!(drill.failed, vec![victim.clone()]);
    assert_eq!(restore_drill(&sink, 2).checked, 2);
    assert_eq!(restore_drill(&sink, 0).checked, 0);
}

#[test]
fn storing_what_is_there_changes_nothing_and_a_lost_card_reads_as_gone() {
    let d = tempfile::tempdir().unwrap();
    let (sink, _) = FolderSink::create(d.path(), "card").unwrap();
    let h = sha(b"same");
    sink.put(&h, b"same").unwrap();
    let path = d.path().join(format!("objects/{}/{h}.hqo", &h[..2]));
    let before = std::fs::read(&path).unwrap();
    sink.put(&h, b"same").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        sink.health(),
        Health {
            reachable: true,
            objects: 1
        }
    );
    std::fs::remove_dir_all(d.path().join("objects")).unwrap();
    assert!(!sink.has(&h));
    assert_eq!(sink.health().objects, 0);
}

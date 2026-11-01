use super::*;
use std::io::Cursor;

const TOP: &str = "o-r-0123abc";
const BLOCK: usize = 512;
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
/// Longer than the 100 bytes a plain tar header holds, so it needs a pax or long-name record.
const LONG_PART: &str =
    "a-folder-name-long-enough-that-the-whole-path-cannot-fit-in-an-old-tar-header";

/// Writes one tar record with the name exactly as given, so hostile names the tar crate's builder
/// would refuse can still be tested.
fn record(
    out: &mut Vec<u8>,
    name: &[u8],
    kind: tar::EntryType,
    mode: u32,
    data: &[u8],
    link: &str,
) {
    let mut header = tar::Header::new_old();
    {
        let old = header.as_old_mut();
        old.name[..name.len()].copy_from_slice(name);
        old.linkname[..link.len()].copy_from_slice(link.as_bytes());
    }
    header.set_entry_type(kind);
    header.set_mode(mode);
    header.set_size(data.len() as u64);
    header.set_cksum();
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
}

fn file(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    record(
        out,
        name.as_bytes(),
        tar::EntryType::Regular,
        0o644,
        data,
        "",
    );
}

/// One pax line: its length counts itself, as the format requires.
fn pax_line(key: &str, value: &str) -> Vec<u8> {
    let rest = format!(" {key}={value}\n");
    let mut len = rest.len() + 1;
    while format!("{len}{rest}").len() != len {
        len += 1;
    }
    format!("{len}{rest}").into_bytes()
}

fn pax(out: &mut Vec<u8>, lines: &[(&str, &str)]) {
    let body: Vec<u8> = lines.iter().flat_map(|(k, v)| pax_line(k, v)).collect();
    record(out, b"PaxHeader", tar::EntryType::XHeader, 0o644, &body, "");
}

fn finish(mut out: Vec<u8>) -> Vec<u8> {
    out.extend_from_slice(&[0u8; BLOCK * 2]);
    out
}

fn run(bytes: Vec<u8>, limits: Limits, stop: bool) -> (tempfile::TempDir, Result<Unpacked>) {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("checkout");
    fs::create_dir(&dest).unwrap();
    let result = unpack(Cursor::new(bytes), &dest, limits, &AtomicBool::new(stop));
    (root, result)
}

fn unpack_bytes(bytes: Vec<u8>) -> (tempfile::TempDir, Result<Unpacked>) {
    run(bytes, Limits::default(), false)
}

fn err_of(result: Result<Unpacked>) -> String {
    format!("{:#}", result.expect_err("the archive should be refused"))
}

#[test]
fn a_normal_tarball_unpacks_with_modes_hashes_and_folders() {
    let mut tar = Vec::new();
    let top_dir = format!("{TOP}/");
    record(
        &mut tar,
        top_dir.as_bytes(),
        tar::EntryType::Directory,
        0o755,
        b"",
        "",
    );
    let global = pax_line("comment", SHA);
    record(
        &mut tar,
        b"pax_global_header",
        tar::EntryType::XGlobalHeader,
        0o644,
        &global,
        "",
    );
    file(&mut tar, &format!("{TOP}/README.md"), b"hello");
    let run_path = format!("{TOP}/bin/run");
    record(
        &mut tar,
        run_path.as_bytes(),
        tar::EntryType::Regular,
        0o755,
        b"#!/bin/sh\n",
        "",
    );
    let empty = format!("{TOP}/empty/");
    record(
        &mut tar,
        empty.as_bytes(),
        tar::EntryType::Directory,
        0o755,
        b"",
        "",
    );
    let (root, result) = unpack_bytes(finish(tar));
    let unpacked = result.unwrap();

    let paths: Vec<&str> = unpacked.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["README.md", "bin/run"]);
    assert_eq!(unpacked.files[0].mode, MODE_REGULAR);
    assert_eq!(unpacked.files[1].mode, MODE_EXECUTABLE);
    assert_eq!(unpacked.files[0].size, 5);
    assert_eq!(
        unpacked.files[0].sha256,
        hex::encode(Sha256::digest(b"hello"))
    );
    let dest = root.path().join("checkout");
    assert_eq!(fs::read(dest.join("README.md")).unwrap(), b"hello");
    assert!(dest.join("empty").is_dir());
    assert!(unpacked.skipped.is_empty());
}

#[test]
fn a_long_path_from_a_pax_record_is_used() {
    let mut tar = Vec::new();
    let long = format!("{TOP}/{LONG_PART}/{LONG_PART}/deep.txt");
    pax(&mut tar, &[("path", &long)]);
    file(&mut tar, &format!("{TOP}/truncated-name"), b"deep");
    let (root, result) = unpack_bytes(finish(tar));
    let unpacked = result.unwrap();
    let rel = format!("{LONG_PART}/{LONG_PART}/deep.txt");
    assert_eq!(unpacked.files[0].path, rel);
    assert_eq!(
        fs::read(root.path().join("checkout").join(&rel)).unwrap(),
        b"deep"
    );
}

#[test]
fn a_long_path_from_a_gnu_record_is_used() {
    let mut tar = Vec::new();
    let mut builder = tar::Builder::new(&mut tar);
    let mut header = tar::Header::new_gnu();
    header.set_size(4);
    header.set_mode(0o644);
    let long = format!("{TOP}/{LONG_PART}/{LONG_PART}/gnu.txt");
    builder
        .append_data(&mut header, &long, &b"long"[..])
        .unwrap();
    builder.finish().unwrap();
    drop(builder);
    let (_root, result) = unpack_bytes(tar);
    assert_eq!(
        result.unwrap().files[0].path,
        format!("{LONG_PART}/{LONG_PART}/gnu.txt")
    );
}

#[test]
fn a_pax_path_that_climbs_out_refuses_the_archive() {
    let mut tar = Vec::new();
    pax(&mut tar, &[("path", &format!("{TOP}/../../escaped"))]);
    file(&mut tar, &format!("{TOP}/innocent"), b"x");
    let (root, result) = unpack_bytes(finish(tar));
    assert!(err_of(result).contains("leaves its folder"));
    assert!(!root.path().join("escaped").exists());
}

#[test]
fn a_pax_size_override_refuses_the_archive() {
    let mut tar = Vec::new();
    pax(&mut tar, &[("size", "999999")]);
    file(&mut tar, &format!("{TOP}/a"), b"x");
    let (_root, result) = unpack_bytes(finish(tar));
    assert!(err_of(result).contains("overrides a file size"));
}

#[test]
fn an_oversized_name_record_is_refused_before_its_body_is_read() {
    let mut header = tar::Header::new_gnu();
    header.as_gnu_mut().unwrap().name[..13].copy_from_slice(b"././@LongLink");
    header.set_entry_type(tar::EntryType::GNULongName);
    header.set_size(8 * 1024 * 1024 * 1024);
    header.set_cksum();
    // Only the header: a reader that tried to read the declared body would hit the end instead.
    let tar = header.as_bytes().to_vec();
    let (_root, result) = unpack_bytes(tar);
    assert!(err_of(result).contains("oversized"));
}

#[test]
fn a_path_that_climbs_out_refuses_the_archive_and_writes_nothing_outside() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/../escaped"), b"x");
    let (root, result) = unpack_bytes(finish(tar));
    assert!(err_of(result).contains("leaves its folder"));
    assert!(!root.path().join("escaped").exists());
}

#[test]
fn an_absolute_path_refuses_the_archive() {
    let mut tar = Vec::new();
    file(&mut tar, "/tmp/hq-absolute-escape", b"x");
    let (_root, result) = unpack_bytes(finish(tar));
    assert!(err_of(result).contains("leaves its folder"));
    assert!(!Path::new("/tmp/hq-absolute-escape").exists());
}

#[test]
fn links_devices_and_non_text_names_are_skipped_never_created() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/ok.txt"), b"ok");
    let sym = format!("{TOP}/sym");
    record(
        &mut tar,
        sym.as_bytes(),
        tar::EntryType::Symlink,
        0o777,
        b"",
        "/etc/passwd",
    );
    let hard = format!("{TOP}/hard");
    record(
        &mut tar,
        hard.as_bytes(),
        tar::EntryType::Link,
        0o644,
        b"",
        "../../outside",
    );
    let fifo = format!("{TOP}/fifo");
    record(
        &mut tar,
        fifo.as_bytes(),
        tar::EntryType::Fifo,
        0o644,
        b"",
        "",
    );
    let mut bad = format!("{TOP}/bad-").into_bytes();
    bad.push(0xff);
    record(&mut tar, &bad, tar::EntryType::Regular, 0o644, b"x", "");
    let (root, result) = unpack_bytes(finish(tar));
    let unpacked = result.unwrap();
    assert_eq!(unpacked.skipped.len(), 4);
    assert_eq!(&unpacked.skipped[..3], ["sym", "hard", "fifo"]);
    let dest = root.path().join("checkout");
    for name in ["sym", "hard", "fifo"] {
        assert!(
            fs::symlink_metadata(dest.join(name)).is_err(),
            "{name} was created"
        );
    }
    assert_eq!(
        fs::read_dir(&dest).unwrap().count(),
        1,
        "only ok.txt is written"
    );
}

#[test]
fn git_metadata_under_any_spelling_is_skipped() {
    let mut tar = Vec::new();
    for name in [
        ".git/hooks/pre-commit",
        "sub/.GIT/config",
        ".git./x",
        "GIT~1/hooks/x",
    ] {
        file(&mut tar, &format!("{TOP}/{name}"), b"rm -rf ~");
    }
    let (root, result) = unpack_bytes(finish(tar));
    let unpacked = result.unwrap();
    assert_eq!(unpacked.skipped.len(), 4);
    assert!(unpacked.files.is_empty());
    assert_eq!(
        fs::read_dir(root.path().join("checkout")).unwrap().count(),
        0
    );
}

#[test]
fn a_second_top_folder_refuses_the_archive() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/a"), b"x");
    file(&mut tar, "other-top/b", b"x");
    let (_root, result) = unpack_bytes(finish(tar));
    assert!(err_of(result).contains("single top folder"));
}

#[test]
fn a_name_already_taken_keeps_the_first_file_and_reports_the_second() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/a"), b"first");
    file(&mut tar, &format!("{TOP}/a"), b"second");
    let (root, result) = unpack_bytes(finish(tar));
    let unpacked = result.unwrap();
    assert_eq!(unpacked.skipped, ["a"]);
    assert_eq!(unpacked.files.len(), 1);
    assert_eq!(fs::read(root.path().join("checkout/a")).unwrap(), b"first");
}

#[test]
fn the_written_size_cap_refuses_the_archive() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/big"), b"12345");
    let small = Limits {
        max_unpacked_bytes: 4,
        ..Limits::default()
    };
    let (_root, result) = run(finish(tar), small, false);
    assert!(err_of(result).contains("unpacks to more than"));
}

#[test]
fn skipped_bodies_count_toward_the_stream_cap() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/.git/big"), &[7u8; 4 * BLOCK]);
    // The header and the end blocks fit in four blocks; only counting the body exceeds them.
    let small = Limits {
        max_stream_bytes: 4 * BLOCK as u64,
        ..Limits::default()
    };
    let (_root, result) = run(finish(tar), small, false);
    assert!(err_of(result).contains("decompresses to more than"));
}

#[test]
fn global_headers_count_toward_the_record_cap() {
    let mut tar = Vec::new();
    for _ in 0..3 {
        record(
            &mut tar,
            b"g",
            tar::EntryType::XGlobalHeader,
            0o644,
            b"",
            "",
        );
    }
    let few = Limits {
        max_records: 2,
        ..Limits::default()
    };
    let (_root, result) = run(finish(tar), few, false);
    assert!(err_of(result).contains("more than 2 records"));
}

#[test]
fn a_stopped_unpack_ends_at_the_next_entry() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/a"), b"x");
    let (_root, result) = run(finish(tar), Limits::default(), true);
    assert!(err_of(result).contains("stopped"));
}

fn ustar(name: &str, prefix: &str, data: &[u8]) -> Vec<u8> {
    let mut header = tar::Header::new_ustar();
    {
        let ustar = header.as_ustar_mut().unwrap();
        ustar.name[..name.len()].copy_from_slice(name.as_bytes());
        ustar.prefix[..prefix.len()].copy_from_slice(prefix.as_bytes());
    }
    header.set_entry_type(tar::EntryType::Regular);
    header.set_mode(0o644);
    header.set_size(data.len() as u64);
    header.set_cksum();
    let mut out = header.as_bytes().to_vec();
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
    finish(out)
}

#[test]
fn a_ustar_prefix_is_joined_and_checked() {
    let (_root, result) = unpack_bytes(ustar("file.txt", &format!("{TOP}/{LONG_PART}"), b"x"));
    assert_eq!(
        result.unwrap().files[0].path,
        format!("{LONG_PART}/file.txt")
    );

    let (_root, result) = unpack_bytes(ustar("x", &format!("{TOP}/../.."), b"x"));
    assert!(err_of(result).contains("leaves its folder"));
}

#[test]
fn a_file_and_a_folder_with_one_name_keep_the_first_and_report_the_rest() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/foo"), b"file");
    file(&mut tar, &format!("{TOP}/foo/bar"), b"under a file");
    let dir = format!("{TOP}/foo/");
    record(
        &mut tar,
        dir.as_bytes(),
        tar::EntryType::Directory,
        0o755,
        b"",
        "",
    );
    let (root, result) = unpack_bytes(finish(tar));
    let unpacked = result.unwrap();
    assert_eq!(unpacked.skipped, ["foo/bar", "foo"]);
    assert_eq!(fs::read(root.path().join("checkout/foo")).unwrap(), b"file");
}

#[test]
fn the_base_record_sits_beside_the_checkout() {
    assert_eq!(
        base_record_path(Path::new("/x/clones/o/r")),
        PathBuf::from("/x/clones/o/r.hq-base.json")
    );
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(bytes).unwrap();
    enc.finish().unwrap()
}

async fn serve(body_sha: &str, tarball: Vec<u8>) -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};
    let server = wiremock::MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/commits/HEAD"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body_sha))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/o/r/tarball/{SHA}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(tarball))
        .mount(&server)
        .await;
    server
}

fn hq_leftovers(dir: &Path) -> usize {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".hq-"))
        .count()
}

#[tokio::test]
async fn fetch_resolves_the_commit_unpacks_and_records_the_base() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/README.md"), b"hello");
    let server = serve(&format!("{SHA}\n"), gzip(&finish(tar))).await;
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("o/r");

    let fetched = fetch(&format!("{}/", server.uri()), "o/r", &dest)
        .await
        .unwrap();
    assert_eq!(fetched.base.commit, SHA);
    assert_eq!(fs::read(dest.join("README.md")).unwrap(), b"hello");
    assert_eq!(load_base(&dest).unwrap(), fetched.base);
    assert_eq!(hq_leftovers(&root.path().join("o")), 0);
}

#[tokio::test]
async fn fetch_leaves_nothing_when_the_archive_is_refused() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/../escaped"), b"x");
    let server = serve(SHA, gzip(&finish(tar))).await;
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("o/r");

    assert!(
        fetch(&format!("{}/", server.uri()), "o/r", &dest)
            .await
            .is_err()
    );
    assert!(!dest.exists());
    assert_eq!(fs::read_dir(root.path().join("o")).unwrap().count(), 0);
}

#[tokio::test]
async fn fetch_replaces_a_stale_record_left_beside_a_missing_checkout() {
    let mut tar = Vec::new();
    file(&mut tar, &format!("{TOP}/new.txt"), b"new");
    let server = serve(SHA, gzip(&finish(tar))).await;
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("o/r");
    fs::create_dir_all(root.path().join("o")).unwrap();
    fs::write(
        base_record_path(&dest),
        br#"{"repo":"o/r","commit":"old","files":[]}"#,
    )
    .unwrap();

    fetch(&format!("{}/", server.uri()), "o/r", &dest)
        .await
        .unwrap();
    let base = load_base(&dest).unwrap();
    assert_eq!(base.commit, SHA);
    assert_eq!(base.files[0].path, "new.txt");
}

#[tokio::test]
async fn a_fetch_dropped_mid_download_leaves_nothing_behind() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};
    let server = wiremock::MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/commits/HEAD"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SHA))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/repos/o/r/tarball/{SHA}")))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("o/r");

    let api = format!("{}/", server.uri());
    let cut = tokio::time::timeout(Duration::from_millis(300), fetch(&api, "o/r", &dest)).await;
    assert!(
        cut.is_err(),
        "the fetch should still be waiting on the slow tarball"
    );
    assert!(!dest.exists());
    assert_eq!(hq_leftovers(&root.path().join("o")), 0);
}

#[tokio::test]
async fn fetch_refuses_an_answer_that_is_not_a_commit_id() {
    let server = serve("<html>not a sha</html>", Vec::new()).await;
    let root = tempfile::tempdir().unwrap();
    let err = fetch(
        &format!("{}/", server.uri()),
        "o/r",
        &root.path().join("o/r"),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("did not return a commit id"));
}

/// Serves `/tarball` as a redirect to `location` and `/real` as the file, answering 418 to any
/// request that still carries a token.
async fn redirecting(server: &wiremock::MockServer, location: &str) {
    use wiremock::matchers::{header_exists, path};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(header_exists("authorization"))
        .and(path("/real"))
        .respond_with(ResponseTemplate::new(418))
        .with_priority(1)
        .mount(server)
        .await;
    Mock::given(path("/real"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"tar".to_vec()))
        .mount(server)
        .await;
    Mock::given(path("/tarball"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", location))
        .mount(server)
        .await;
}

async fn download_with_token(api: &wiremock::MockServer) -> Result<()> {
    let root = tempfile::tempdir().unwrap();
    let req = client()
        .unwrap()
        .get(format!("{}/tarball", api.uri()))
        .bearer_auth("secret-read-token");
    download(req, &root.path().join("t.tar.gz")).await
}

#[tokio::test]
async fn a_redirect_to_another_host_never_carries_the_token() {
    // GitHub sends the tarball from codeload.github.com, another host than the API.
    let target = wiremock::MockServer::start().await;
    redirecting(&target, "/unused").await;
    let api = wiremock::MockServer::start().await;
    redirecting(&api, &format!("{}/real", target.uri())).await;
    download_with_token(&api).await.unwrap();

    // The control: a redirect on the same origin keeps the token, and the 418 shows it.
    let same = wiremock::MockServer::start().await;
    redirecting(&same, &format!("{}/real", same.uri())).await;
    assert!(
        download_with_token(&same)
            .await
            .unwrap_err()
            .to_string()
            .contains("418")
    );
}

#[test]
fn old_leftovers_are_swept_and_fresh_ones_kept() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join(format!("{STAGING_PREFIX}old"));
    let fresh = root.path().join(format!("{ARCHIVE_PREFIX}fresh"));
    let other = root.path().join("r");
    fs::create_dir(&old).unwrap();
    fs::write(&fresh, b"x").unwrap();
    fs::create_dir(&other).unwrap();
    let long_ago = std::time::SystemTime::now() - STALE_AFTER * 2;
    filetime::set_file_mtime(&old, filetime::FileTime::from_system_time(long_ago)).unwrap();
    sweep_leftovers(root.path());
    assert!(!old.exists());
    assert!(fresh.exists());
    assert!(other.exists());
}

/// Opt-in: `cargo test -p hq-tools github::tarball -- --ignored` reads a real public repo.
#[tokio::test]
#[ignore = "reads api.github.com; run on demand"]
async fn fetch_a_real_public_repo() {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("CalvinMagezi/Offline-Botanist");
    let fetched = fetch(
        "https://api.github.com/",
        "CalvinMagezi/Offline-Botanist",
        &dest,
    )
    .await
    .unwrap();
    assert!(fetched.base.files.iter().any(|f| f.path == "README.md"));
    assert!(dest.join("packages/botany_core/pubspec.yaml").is_file());
}

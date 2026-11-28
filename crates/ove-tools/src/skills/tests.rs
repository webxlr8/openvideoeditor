use super::*;

fn store() -> (tempfile::TempDir, SkillStore) {
    let d = tempfile::tempdir().unwrap();
    let s = SkillStore::of_library(d.path());
    (d, s)
}

fn put(s: &SkillStore, id: &str, text: &str) {
    std::fs::create_dir_all(s.dir(id)).unwrap();
    std::fs::write(s.dir(id).join(SKILL_FILE), text).unwrap();
}

const GOOD: &str = "---\nname: Tidy cuts\ndescription: Close small gaps on the main track.\n---\n\nStep one.\n";

#[test]
fn a_valid_skill_is_listed_with_its_front_matter() {
    let (_d, s) = store();
    put(&s, "tidy-cuts", GOOD);
    let all = s.list();
    let k = all.iter().find(|k| k.id == "tidy-cuts").unwrap();
    assert!(k.is_valid(), "{:?}", k.invalid);
    assert_eq!(k.name, "Tidy cuts");
    assert_eq!(k.description, "Close small gaps on the main track.");
    assert_eq!(k.origin, Origin::Local);
    assert!(!k.builtin && !k.modified);
    assert_eq!(k.body().trim(), "Step one.");
    assert!(s.valid().iter().any(|k| k.id == "tidy-cuts"));
}

#[test]
fn builtin_skills_are_listed_first_and_are_valid() {
    let (_d, s) = store();
    let all = s.list();
    let ids: Vec<&str> = all.iter().take(BUILTINS.len()).map(|k| k.id.as_str()).collect();
    assert_eq!(ids, ["caption-styling", "colour-matching", "interview-cutting"]);
    for k in &all[..BUILTINS.len()] {
        assert!(k.builtin && k.is_valid(), "{}: {:?}", k.id, k.invalid);
        assert_eq!(k.origin, Origin::Builtin);
    }
    // Nothing is written for them.
    assert!(!s.root.exists());
}

#[test]
fn invalid_and_missing_field_skills_are_listed_with_the_reason_and_never_offered() {
    let (_d, s) = store();
    put(&s, "no-description", "---\nname: X\n---\nBody\n");
    put(&s, "no-name", "---\ndescription: Y\n---\nBody\n");
    put(&s, "no-front-matter", "Just text\n");
    put(&s, "unclosed", "---\nname: X\ndescription: Y\nBody\n");
    put(&s, "empty-body", "---\nname: X\ndescription: Y\n---\n\n");
    put(&s, "bad-line", "---\nname X\ndescription: Y\n---\nBody\n");
    std::fs::create_dir_all(s.dir("no-file")).unwrap();
    put(&s, "Upper", GOOD);
    put(&s, "caption-styling", GOOD);
    let all = s.list();
    let why = |id: &str| all.iter().find(|k| k.id == id).and_then(|k| k.invalid.clone()).unwrap_or_default();
    assert!(why("no-description").contains("description"), "{}", why("no-description"));
    assert!(why("no-name").contains("name"));
    assert!(why("no-front-matter").contains("front matter"));
    assert!(why("unclosed").contains("front matter"));
    assert!(why("empty-body").contains("no text"));
    assert!(why("bad-line").contains("line 2"), "{}", why("bad-line"));
    assert!(why("no-file").contains("SKILL.md"));
    assert!(why("Upper").contains("id"));
    // A folder may not shadow a builtin skill.
    assert!(all.iter().filter(|k| k.id == "caption-styling").any(|k| !k.builtin && !k.is_valid()));
    let offered: Vec<String> = s.valid().into_iter().map(|k| k.id).collect();
    assert_eq!(offered, ["caption-styling", "colour-matching", "interview-cutting"]);
    // Reading an invalid skill names why; an unknown one lists the installed ids.
    assert_eq!(s.get("no-name").unwrap_err().code, "skill.invalid");
    let e = s.get("nope").unwrap_err();
    assert_eq!(e.code, "skill.unknown");
    assert_eq!(e.details["installed"][0], "caption-styling");
}

#[test]
fn front_matter_values_may_be_quoted_folded_and_crlf() {
    let fm = parse_front_matter(
        "---\r\nname: \"Quoted: name\"\r\ndescription: >\r\n  First line\r\n  second line\r\nallowed-tools: Bash\r\n---\r\nBody\r\n",
    )
    .unwrap();
    assert_eq!(fm.name.as_deref(), Some("Quoted: name"));
    assert_eq!(fm.description.as_deref(), Some("First line second line"));
    assert_eq!(fm.other_keys, ["allowed-tools"]);
}

#[test]
fn long_names_descriptions_and_files_are_refused() {
    let long_name = format!("---\nname: {}\ndescription: d\n---\nb\n", "n".repeat(MAX_NAME_CHARS + 1));
    assert!(validate_text(&long_name).unwrap_err().contains("name"));
    let long_desc = format!("---\nname: n\ndescription: {}\n---\nb\n", "d".repeat(MAX_DESCRIPTION_CHARS + 1));
    assert!(validate_text(&long_desc).unwrap_err().contains("description"));
    let big = format!("---\nname: n\ndescription: d\n---\n{}\n", "b".repeat(MAX_SKILL_BYTES));
    assert!(validate_text(&big).unwrap_err().contains("larger"));
}

#[test]
fn ids_are_folder_safe() {
    for ok in ["a", "tidy-cuts", "v2", "0-start"] {
        assert!(valid_id(ok), "{ok}");
    }
    for bad in ["", "-a", "A", "a b", "a/b", "..", "a.b", "c:", &"x".repeat(65)] {
        assert!(!valid_id(bad), "{bad}");
    }
    assert_eq!(id_from_name("Tidy the Cuts!"), "tidy-the-cuts");
    assert_eq!(id_from_name("***"), "skill");
}

#[test]
fn writes_record_the_hash_and_origin_and_builtins_are_protected() {
    let (_d, s) = store();
    let k = s.write("tidy-cuts", GOOD, Origin::Assistant, None).unwrap();
    assert_eq!(k.origin, Origin::Assistant);
    assert_eq!(s.ledger().skills["tidy-cuts"].sha256, sha256_hex(GOOD.as_bytes()));
    assert!(!k.modified);
    // An edit outside the app shows as modified.
    std::fs::write(s.dir("tidy-cuts").join(SKILL_FILE), GOOD.replace("one", "two")).unwrap();
    assert!(s.get("tidy-cuts").unwrap().modified);
    assert_eq!(s.write("caption-styling", GOOD, Origin::Local, None).unwrap_err().code, "skill.builtin");
    assert_eq!(s.remove("caption-styling").unwrap_err().code, "skill.builtin");
    assert_eq!(s.write("bad id", GOOD, Origin::Local, None).unwrap_err().code, "skill.id");
    assert_eq!(s.write("x", "no front matter", Origin::Local, None).unwrap_err().code, "skill.invalid");
    s.remove("tidy-cuts").unwrap();
    assert!(!s.dir("tidy-cuts").exists());
    assert!(!s.ledger().skills.contains_key("tidy-cuts"));
    assert_eq!(s.remove("tidy-cuts").unwrap_err().code, "skill.unknown");
    assert_eq!(s.remove("../x").unwrap_err().code, "skill.unknown");
}

#[test]
fn the_persons_edit_of_a_catalog_skill_shows_as_modified() {
    let (_d, s) = store();
    s.write("pacing", GOOD, Origin::Catalog { url: "https://h.example/c.json".into() }, Some("pacing".into())).unwrap();
    let k = s.save_edit("pacing", &GOOD.replace("one", "two"), Origin::Local).unwrap();
    assert!(k.modified);
    assert_eq!(k.origin, Origin::Catalog { url: "https://h.example/c.json".into() });
    // A new skill saved in the editor is the person's own.
    let n = s.save_edit("mine", GOOD, Origin::Local).unwrap();
    assert_eq!(n.origin, Origin::Local);
    assert!(!n.modified);
    assert_eq!(s.save_edit("caption-styling", GOOD, Origin::Local).unwrap_err().code, "skill.builtin");
    assert_eq!(s.save_edit("mine", "nope", Origin::Local).unwrap_err().code, "skill.invalid");
}

#[test]
fn free_ids_skip_taken_ones_and_builtins() {
    let (_d, s) = store();
    assert_eq!(s.free_id("tidy"), "tidy");
    put(&s, "tidy", GOOD);
    assert_eq!(s.free_id("tidy"), "tidy-2");
    assert_eq!(s.free_id("caption-styling"), "caption-styling-2");
}

fn many(n: usize) -> Vec<Skill> {
    (0..n)
        .map(|i| {
            let text = format!("---\nname: S{i}\ndescription: {} {i}\n---\nbody\n", "word ".repeat(60));
            skill_from(&format!("s{i:02}"), text, Origin::Local, false, None)
        })
        .collect()
}

#[test]
fn the_index_is_one_short_line_per_skill_and_capped_at_60() {
    assert_eq!(index_text(&[]), None);
    let idx = index_text(&many(3)).unwrap();
    let lines: Vec<&str> = idx.lines().filter(|l| l.starts_with("- ")).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with("- s00: word word"));
    // id, ": ", then at most 200 characters of description.
    assert!(lines[0].chars().count() <= 2 + 3 + 2 + INDEX_DESCRIPTION_CHARS, "{}", lines[0]);
    assert!(lines[0].ends_with('…'));
    let idx = index_text(&many(75)).unwrap();
    let lines: Vec<&str> = idx.lines().filter(|l| l.starts_with("- ")).collect();
    assert_eq!(lines.len(), INDEX_CAP + 1);
    assert!(lines[INDEX_CAP].contains("15 more"));
    // Invalid skills are left out.
    let mut v = many(2);
    v[1].invalid = Some("x".into());
    assert_eq!(index_text(&v).unwrap().lines().filter(|l| l.starts_with("- ")).count(), 1);
}

#[test]
fn links_are_not_followed() {
    let (d, s) = store();
    let outside = d.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join(SKILL_FILE), GOOD).unwrap();
    std::fs::create_dir_all(&s.root).unwrap();
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&outside, s.dir("linked")).is_ok();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&outside, s.dir("linked")).is_ok();
    if made {
        assert!(s.list().iter().all(|k| k.id != "linked"));
    }
}

#[test]
fn mcp_prompts_and_resources_follow_the_store() {
    let (_d, s) = store();
    put(&s, "tidy-cuts", GOOD);
    put(&s, "broken", "---\nname: B\n---\nx\n");
    let p = mcp_prompts(&s);
    let names: Vec<&str> = p["prompts"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["caption-styling", "colour-matching", "interview-cutting", "tidy-cuts"]);
    let got = mcp_prompt_get(&s, "tidy-cuts").unwrap();
    assert!(got["messages"][0]["content"]["text"].as_str().unwrap().contains("Step one."));
    let r = mcp_resources(&s);
    assert_eq!(r["resources"][3]["uri"], "ove://skills/tidy-cuts");
    let read = mcp_resource_read(&s, "ove://skills/tidy-cuts").unwrap();
    assert_eq!(read["contents"][0]["text"], GOOD);
    assert_eq!(mcp_resource_read(&s, "ove://skills/broken").unwrap_err().code, "skill.invalid");
    assert_eq!(mcp_resource_read(&s, "file:///etc/passwd").unwrap_err().code, "resource.unknown");
}

#[test]
fn the_builtin_skills_name_only_real_tools() {
    for b in BUILTINS {
        assert_eq!(unknown_tool_names(b.text), Vec::<String>::new(), "{}", b.id);
    }
    assert_eq!(unknown_tool_names("use `print_timeline` then `make_magic` and `x`"), ["make_magic"]);
}

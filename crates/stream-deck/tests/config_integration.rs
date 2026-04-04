#[path = "../src/config.rs"]
mod config;

use config::*;

fn minimal_app() -> AppConfig {
    AppConfig {
        app: AppMeta {
            home: "home".to_string(),
        },
        dashboard: DashboardConfig::default(),
        pages: vec![PageConfig {
            id: "home".to_string(),
            title: "Home".to_string(),
            items: vec![],
        }],
        display: DisplayConfig::default(),
        dynamic: DynamicConfig::default(),
        watch: WatchConfig::default(),
    }
}

fn assert_contains_error(report: &ConfigValidationReport, expected: &str) {
    assert!(
        report.errors.iter().any(|e| e.contains(expected)),
        "expected error keyword not found: {expected}, errors={:?}",
        report.errors
    );
}

fn assert_contains_warning(report: &ConfigValidationReport, expected: &str) {
    assert!(
        report.warnings.iter().any(|w| w.contains(expected)),
        "expected warning keyword not found: {expected}, warnings={:?}",
        report.warnings
    );
}

#[test]
fn test_parse_app_config_with_pages() {
    let text = r#"
[app]
home = "home"

[dashboard]

[[dashboard.sections]]
type = "cpu"
capacity = 10

[[dashboard.sections]]
type = "mem"
capacity = 10

[[dashboard.sections]]
type = "load"
capacity = 10

[[dashboard.sections]]
type = "notif"
capacity = 10

[[pages]]
id = "home"

[[pages.items]]
kind = "nav"
label = "apps"
target = "apps"

[[pages]]
id = "apps"

[[pages.items]]
kind = "command"
label = "terminal"
command = ["xterm"]
"#;
    let parsed: AppConfig = toml::from_str(text).expect("parse app config");
    assert_eq!(parsed.app.home, "home");
    assert_eq!(parsed.pages.len(), 2);
    assert_eq!(parsed.dashboard.sections.len(), 4);
}

#[test]
fn test_validate_rejects_page_size_zero() {
    let mut app = minimal_app();
    app.dynamic.ssh_hosts = Some(DynamicSshConfig {
        source: "hosts.toml".to_string(),
        page_size: 0,
        ..DynamicSshConfig::default()
    });

    let report = validate_app_config(&app, None, true);
    assert!(report.has_errors());
    assert_contains_error(&report, "page_size は 1 以上");
}

#[test]
fn test_validate_rejects_missing_nav_target() {
    let mut app = minimal_app();
    app.pages[0].items = vec![PageItemConfig::Nav {
        label: "go".to_string(),
        target: "missing".to_string(),
        priority: None,
        visible: true,
    }];

    let report = validate_app_config(&app, None, true);
    assert_contains_error(&report, "nav target が存在しません");
}

#[test]
fn test_validate_disables_ssh_when_terminal_title_not_capable() {
    let mut app = minimal_app();
    app.dynamic.ssh_hosts = Some(DynamicSshConfig {
        source: "hosts.toml".to_string(),
        ..DynamicSshConfig::default()
    });

    let report = validate_app_config(&app, None, false);
    assert!(!report.ssh_enabled);
    assert!(report.ssh_disabled_reason.is_some());
    assert!(!report.warnings.is_empty());
}

#[test]
fn test_validate_error_cases_table() {
    let mut dup_page = minimal_app();
    dup_page.pages.push(PageConfig {
        id: "home".to_string(),
        title: "dup".to_string(),
        items: vec![],
    });

    let mut empty_command = minimal_app();
    empty_command.pages[0].items = vec![PageItemConfig::Command {
        label: "bad".to_string(),
        command: vec![],
        priority: None,
    }];

    let mut missing_home = minimal_app();
    missing_home.app.home = "missing".to_string();

    let cases: Vec<(AppConfig, &str)> = vec![
        (dup_page, "重複した page id"),
        (empty_command, "command が空"),
        (missing_home, "home ページが存在しません"),
    ];

    for (app, expected_error) in cases {
        let report = validate_app_config(&app, None, true);
        assert_contains_error(&report, expected_error);
    }
}

#[test]
fn test_validate_rejects_duplicate_ssh_host_id() {
    let mut app = minimal_app();
    app.dynamic.ssh_hosts = Some(DynamicSshConfig {
        source: "hosts.toml".to_string(),
        ..DynamicSshConfig::default()
    });
    let hosts = vec![
        HostEntry {
            id: "cam-01".to_string(),
            label: "cam01".to_string(),
            host: "user@192.168.1.10".to_string(),
            tags: vec![],
        },
        HostEntry {
            id: "cam-01".to_string(),
            label: "cam01-dup".to_string(),
            host: "user@192.168.1.11".to_string(),
            tags: vec![],
        },
    ];

    let report = validate_app_config(&app, Some(hosts.as_slice()), true);
    assert_contains_error(&report, "重複した SSH host id");
}

#[test]
fn test_load_ssh_hosts_entries_normalizes_values() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let layout = root.join("layout.toml");
    let hosts = root.join("ssh_hosts.toml");

    std::fs::write(&layout, "[app]\nhome=\"home\"\n").expect("write layout");
    std::fs::write(
        &hosts,
        r#"
[[hosts]]
id = " cam-01 "
label = " Cam01 "
host = " user@192.168.1.10 "
tags = [" edge ", "", " field "]
"#,
    )
    .expect("write hosts");

    let loaded = load_ssh_hosts_entries(
        layout.to_str().expect("layout path"),
        hosts.to_str().expect("hosts path"),
    )
    .expect("load hosts");

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, "cam-01");
    assert_eq!(loaded[0].label, "Cam01");
    assert_eq!(loaded[0].host, "user@192.168.1.10");
    assert_eq!(
        loaded[0].tags,
        vec!["edge".to_string(), "field".to_string()]
    );
}

#[test]
fn test_build_ssh_page_build_input_from_dynamic_config() {
    let mut app = minimal_app();
    app.dynamic.ssh_hosts = Some(DynamicSshConfig {
        source: "ssh_hosts.toml".to_string(),
        page_size: 4,
        page_id_prefix: "ssh_hosts".to_string(),
        terminal: vec!["/usr/bin/terminator".to_string()],
        ssh_template: "ssh {host}".to_string(),
    });

    let input = build_ssh_page_build_input(&app).expect("build input");
    assert_eq!(input.page_size, 4);
    assert_eq!(input.page_id_prefix, "ssh_hosts");
    assert_eq!(input.terminal, vec!["/usr/bin/terminator".to_string()]);
    assert_eq!(input.ssh_template, "ssh {host}");
}

#[test]
fn test_build_dynamic_ssh_pages_chunks_hosts_and_adds_nav_items() {
    let input = SshPageBuildInput {
        page_size: 2,
        page_id_prefix: "ssh_hosts".to_string(),
        terminal: vec!["/usr/bin/terminator".to_string()],
        ssh_template: "ssh {host}".to_string(),
    };
    let hosts = vec![
        HostEntry {
            id: "cam-01".to_string(),
            label: "Cam01".to_string(),
            host: "user@192.168.1.10".to_string(),
            tags: vec![],
        },
        HostEntry {
            id: "cam-02".to_string(),
            label: "Cam02".to_string(),
            host: "user@192.168.1.11".to_string(),
            tags: vec![],
        },
        HostEntry {
            id: "cam-03".to_string(),
            label: "Cam03".to_string(),
            host: "user@192.168.1.12".to_string(),
            tags: vec![],
        },
    ];

    let pages = build_dynamic_ssh_pages(&input, &hosts);
    assert_eq!(pages.len(), 2);
    assert_eq!(pages[0].id, "ssh_hosts");
    assert_eq!(pages[1].id, "ssh_hosts_2");

    let has_next = pages[0]
        .items
        .iter()
        .any(|item| matches!(item, PageItemConfig::Nav { label, target, .. } if label == "Next" && target == "ssh_hosts_2"));
    assert!(has_next, "first page should contain Next nav item");

    let has_prev = pages[1]
        .items
        .iter()
        .any(|item| matches!(item, PageItemConfig::Nav { label, target, .. } if label == "Prev" && target == "ssh_hosts"));
    assert!(has_prev, "second page should contain Prev nav item");

    let ssh_item_count_page1 = pages[0]
        .items
        .iter()
        .filter(|item| matches!(item, PageItemConfig::SshConnect { .. }))
        .count();
    let ssh_item_count_page2 = pages[1]
        .items
        .iter()
        .filter(|item| matches!(item, PageItemConfig::SshConnect { .. }))
        .count();
    assert_eq!(ssh_item_count_page1, 2);
    assert_eq!(ssh_item_count_page2, 1);
}

#[test]
fn test_validate_warns_out_of_range_item_priority() {
    let mut app = minimal_app();
    app.pages[0].items = vec![PageItemConfig::Back {
        label: "Back".to_string(),
        priority: Some(PAGE_ITEM_PRIORITY_RECOMMENDED_MAX + 1),
    }];

    let report = validate_app_config(&app, None, true);
    assert_contains_warning(&report, "priority=");
    assert_contains_warning(&report, "推奨レンジ外");
}

#[test]
fn test_bundle_load_reads_hosts_and_watch_targets() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let layout = root.join("layout.toml");
    let hosts = root.join("ssh_hosts.toml");

    std::fs::write(
        &layout,
        r#"
[app]
home = "home"

[dashboard]

[[dashboard.sections]]
type = "cpu"
capacity = 10

[[dashboard.sections]]
type = "mem"
capacity = 10

[[dashboard.sections]]
type = "load"
capacity = 10

[[dashboard.sections]]
type = "notif"
capacity = 10

[[pages]]
id = "home"

[watch]
includes = ["extra.toml"]

[dynamic.ssh_hosts]
source = "ssh_hosts.toml"
page_size = 6
"#,
    )
    .expect("write layout");
    std::fs::write(
        &hosts,
        r#"
[[hosts]]
id = "cam-01"
label = "cam01"
host = "user@192.168.1.10"
"#,
    )
    .expect("write hosts");

    let bundle =
        AppConfigBundle::load(layout.to_str().expect("path str"), true).expect("bundle load");

    assert!(
        bundle.report.errors.is_empty(),
        "errors={:?}",
        bundle.report.errors
    );
    assert!(bundle.ssh_hosts.is_some(), "ssh_hosts should be loaded");
    let ssh_path = std::fs::canonicalize(&hosts).expect("canonical hosts");
    assert!(
        bundle.watch_targets.contains(&ssh_path),
        "ssh hosts file should be in watch targets"
    );
}

#[test]
fn test_resolve_watch_targets_includes_ssh_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let layout = root.join("layout.toml");
    let hosts = root.join("ssh_hosts.toml");
    std::fs::write(&layout, "").expect("write layout");
    std::fs::write(&hosts, "").expect("write hosts");

    let mut app = AppConfig::default();
    app.dynamic.ssh_hosts = Some(DynamicSshConfig {
        source: "ssh_hosts.toml".to_string(),
        ..DynamicSshConfig::default()
    });

    let targets = resolve_watch_targets(layout.to_str().expect("path str"), &app);
    let hosts_path = std::fs::canonicalize(&hosts).expect("canonical hosts");
    assert!(targets.contains(&hosts_path));
}

#[test]
fn test_legacy_layout_format_is_rejected() {
    let old_text = r#"
[[layout.sections]]
type = "cpu"
capacity = 10
"#;
    let parsed = toml::from_str::<AppConfig>(old_text);
    assert!(parsed.is_err(), "legacy layout format must be rejected");
}

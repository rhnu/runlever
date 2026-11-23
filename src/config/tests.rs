use std::collections::BTreeMap;

use serde_json::json;

use super::{Catalog, ResolveError};

const EXAMPLE: &str = r#"
[server]
admin_token = "test-admin"

[enums.projects]
zulu = {label = "Zulu", value = "/opt/project with spaces"}
alpha = {label = "Alpha", value = "/opt/alpha"}

[enums.operations]
restart = {label = "Restart", value = "compose-restart"}

[actions.restart-zulu]
label = "Restart Zulu"
call = "restart"
bind = {z-project = "zulu"}

[actions.run]
label = "Run"
inputs = {z-project = {enum = "projects", label = "Project"}, a-operation = "operations"}
lock = {param = "z-project"}
exec = {program = "tool", args = [{param = "z-project"}, "--", {param = "a-operation"}]}
webhook = {token = "test-run-hook"}

[actions.restart]
label = "Restart"
call = "run"
bind = {a-operation = "restart"}
webhook = {token = "test-restart-hook"}

[actions.command]
label = "Run shell command"
inputs = {project = "projects"}
lock = {param = "project"}
exec = {command = 'exec tool "$1"', cwd = {param = "project"}, args = [{param = "project"}]}
"#;

fn selections(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, option)| (name.to_string(), option.to_string()))
        .collect()
}

#[test]
fn exposes_only_public_action_metadata_in_declaration_order() {
    let catalog = Catalog::parse(EXAMPLE).unwrap();
    let views = catalog.actions();
    let ids: Vec<_> = views.iter().map(|action| action.id.as_str()).collect();
    assert_eq!(ids, ["restart-zulu", "run", "restart", "command"]);
    assert_eq!(
        serde_json::to_value(&views[1]).unwrap(),
        json!({
            "id": "run",
            "label": "Run",
            "inputs": [
                {
                    "id": "z-project",
                    "label": "Project",
                    "options": [{"id": "zulu", "label": "Zulu"}, {"id": "alpha", "label": "Alpha"}]
                },
                {
                    "id": "a-operation",
                    "label": "a-operation",
                    "options": [{"id": "restart", "label": "Restart"}]
                }
            ]
        })
    );
    assert!(views[0].inputs.is_empty());
    assert_eq!(views[2].inputs.len(), 1);
    assert_eq!(views[2].inputs[0].id, "z-project");
    assert_eq!(views[2].inputs[0].label, "Project");
    assert_eq!(catalog.webhook_token("run"), Some("test-run-hook"));
    assert_eq!(catalog.webhook_token("restart"), Some("test-restart-hook"));
    assert_eq!(catalog.webhook_token("restart-zulu"), None);
}

#[test]
fn resolves_wrapped_argv_and_shares_locks_by_enum_and_option() {
    let catalog = Catalog::parse(EXAMPLE).unwrap();
    let wrapped = catalog.resolve("restart-zulu", &BTreeMap::new()).unwrap();
    assert_eq!(wrapped.action, "restart-zulu");
    assert_eq!(wrapped.label, "Restart Zulu");
    assert_eq!(wrapped.target, "run");
    assert_eq!(wrapped.program, "tool");
    assert_eq!(
        wrapped.args,
        ["/opt/project with spaces", "--", "compose-restart"]
    );
    assert!(wrapped.cwd.is_none());
    assert_eq!(
        wrapped.selections,
        selections(&[("z-project", "zulu"), ("a-operation", "restart")])
    );

    let direct = catalog
        .resolve(
            "run",
            &selections(&[("z-project", "zulu"), ("a-operation", "restart")]),
        )
        .unwrap();
    let shell = catalog
        .resolve("command", &selections(&[("project", "zulu")]))
        .unwrap();
    assert!(wrapped.lock.is_some());
    assert_eq!(wrapped.lock, direct.lock);
    assert_eq!(wrapped.lock, shell.lock);
    let other = catalog
        .resolve("command", &selections(&[("project", "alpha")]))
        .unwrap();
    assert_ne!(shell.lock, other.lock);
    assert_eq!(shell.program, "/bin/sh");
    assert_eq!(
        shell.args,
        [
            "-c",
            "exec tool \"$1\"",
            "runlever",
            "/opt/project with spaces"
        ]
    );
    assert_eq!(shell.cwd.as_deref(), Some("/opt/project with spaces"));
}

#[test]
fn groups_apply_to_direct_and_derived_actions() {
    let source = EXAMPLE
        .replace(
            "[actions.run]\nlabel = \"Run\"",
            "[actions.run]\nlabel = \"Run\"\ngroup = \"compose\"",
        )
        .replace(
            "[actions.command]\nlabel = \"Run shell command\"",
            "[actions.command]\nlabel = \"Run shell command\"\ngroup = \"shortcuts\"",
        );
    let source = format!(
        "{source}\n[groups.compose]\nlabel = 'Compose'\nmax_concurrent = 2\n
         [groups.shortcuts]\nlabel = 'Shortcuts'"
    );
    let catalog = Catalog::parse(&source).unwrap();
    let direct = catalog
        .resolve(
            "run",
            &selections(&[("z-project", "zulu"), ("a-operation", "restart")]),
        )
        .unwrap();
    let derived = catalog.resolve("restart-zulu", &BTreeMap::new()).unwrap();
    for invocation in [&direct, &derived] {
        let group = invocation.group_limit.as_ref().unwrap();
        assert_eq!(group.id, "compose");
        assert_eq!(group.max_concurrent.get(), 2);
    }
    assert_eq!(derived.lock, direct.lock);
    let command = catalog
        .resolve("command", &selections(&[("project", "zulu")]))
        .unwrap();
    assert!(command.group_limit.is_none());
    let views = catalog.actions();
    assert_eq!(
        serde_json::to_value(views[0].group.as_ref()).unwrap(),
        json!({"id": "compose", "label": "Compose"})
    );
    assert_eq!(views[3].group.as_ref().unwrap().id, "shortcuts");
}

#[test]
fn validates_groups_and_queue_limits() {
    let source = "[server]\nadmin_token = 'test-admin'\n";
    let configured = Catalog::parse(&format!(
        "{source}[queue]\nmax_pending = 0\nmax_wait_seconds = 5"
    ))
    .unwrap();
    assert_eq!(configured.queue().max_pending, 0);
    assert_eq!(configured.queue().max_wait_seconds, 5);
    let defaults = Catalog::parse(source).unwrap().queue();
    assert_eq!(defaults.max_pending, 16);
    assert_eq!(defaults.max_wait_seconds, 60);
    assert_error(
        &format!("{source}[queue]\nmax_wait_seconds = 0"),
        "queue.max_wait_seconds must be positive",
    );
    assert_error(
        &format!("{source}[groups.ops]\nlabel = 'Ops'\nmax_concurrent = 0"),
        "groups.ops.max_concurrent",
    );
    let actions = r#"
[actions.base]
label = "Base"
group = "ops"
exec = {program = "tool"}
[actions.derived]
label = "Derived"
group = "other"
call = "base"
"#;
    let groups = "[groups.ops]\nlabel = 'Ops'\n[groups.other]\nlabel = 'Other'";
    assert_error(&format!("{source}{actions}"), "unknown group");
    assert_error(
        &format!("{source}{actions}{groups}"),
        "cannot override inherited group",
    );
    let derived_only = actions.replace("group = \"ops\"\n", "");
    let catalog = Catalog::parse(&format!("{source}{derived_only}{groups}")).unwrap();
    let views = catalog.actions();
    assert!(views[0].group.is_none());
    assert_eq!(views[1].group.as_ref().unwrap().id, "other");
}

#[test]
fn rejects_missing_unknown_and_fixed_inputs() {
    let catalog = Catalog::parse(EXAMPLE).unwrap();
    let error = catalog.resolve("restart", &BTreeMap::new()).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ResolveError>(),
        Some(ResolveError::MissingInput(name)) if name == "z-project"
    ));
    let error = catalog
        .resolve(
            "restart",
            &selections(&[("z-project", "/opt/project with spaces")]),
        )
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ResolveError>(),
        Some(ResolveError::UnknownOption(name)) if name == "z-project"
    ));
    let error = catalog
        .resolve("restart-zulu", &selections(&[("z-project", "alpha")]))
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ResolveError>(),
        Some(ResolveError::UnexpectedInput)
    ));
}

#[test]
fn rejects_invalid_action_references_and_shapes() {
    let invalid_actions = [
        ("call", r#"call = "missing""#, "unknown action"),
        (
            "input",
            r#"call = "run"
bind = {typo = "zulu"}"#,
            "unknown input",
        ),
        (
            "fixed",
            r#"call = "restart"
bind = {a-operation = "restart"}"#,
            "already bound",
        ),
        (
            "option",
            r#"call = "run"
bind = {z-project = "/opt/alpha"}"#,
            "unknown option",
        ),
        (
            "mixed",
            r#"call = "run"
exec = {program = "tool"}"#,
            "exactly one",
        ),
        (
            "exec-mode",
            r#"exec = {program = "tool", command = "tool"}"#,
            "exactly one",
        ),
        (
            "exec-param",
            r#"exec = {program = "tool", args = [{param = "typo"}]}"#,
            "unknown input",
        ),
        (
            "exec-enum",
            r#"inputs = {project = "missing"}
exec = {program = "tool"}"#,
            "unknown enum",
        ),
        (
            "lock-param",
            r#"exec = {program = "tool"}
lock = {param = "missing"}"#,
            "unknown input",
        ),
        (
            "input-id",
            r#"inputs = {"a/b" = "projects"}
exec = {program = "tool"}"#,
            "invalid input name",
        ),
    ];
    for (id, definition, expected) in invalid_actions {
        let source = format!("{EXAMPLE}\n[actions.invalid-{id}]\nlabel = 'Invalid'\n{definition}");
        assert_error(&source, expected);
    }

    let source = r#"
[server]
admin_token = "test-admin"
[actions.first]
label = "First"
call = "second"
[actions.second]
label = "Second"
call = "first"
"#;
    assert_error(source, "cycle");
}

#[test]
fn validates_server_defaults_and_https_origin() {
    let catalog = Catalog::parse(EXAMPLE).unwrap();
    assert_eq!(catalog.listen(), "127.0.0.1:7380".parse().unwrap());
    assert_eq!(catalog.retention_days(), 30);
    assert_eq!(catalog.origin(), "https://localhost");

    let custom = "[server]\nadmin_token = 'test'\norigin = 'https://EXAMPLE.test:443/'";
    let catalog = Catalog::parse(custom).unwrap();
    assert_eq!(catalog.origin(), "https://example.test");
    let source = custom.replace("https://EXAMPLE.test:443/", "http://example.test");
    assert_error(&source, "HTTPS origin");
    let source = custom.replace("https://EXAMPLE.test:443/", "https://example.test/path");
    assert_error(&source, "HTTPS origin");
    assert_error(&format!("{custom}\n[logs]\nretention_days = 0"), "positive");
}

#[test]
fn rejects_invalid_tokens_and_redacts_toml_diagnostics() {
    let source = EXAMPLE.replace("token = \"test-restart-hook\"", "token = \" \"");
    assert_error(&source, "webhook token must be nonempty");
    let source = EXAMPLE.replace("token = \"test-restart-hook\"", "token = \"test-admin\"");
    assert_error(&source, "must differ from the admin token");
    let source = EXAMPLE.replace("token = \"test-restart-hook\"", "token = \"test-run-hook\"");
    assert_error(&source, "must use different webhook tokens");
    let source = EXAMPLE.replace("admin_token = \"test-admin\"", "admin_token = \"\"");
    assert_error(&source, "server.admin_token must be nonempty");

    let source = EXAMPLE.replace(
        "token = \"test-restart-hook\"",
        "token = \"test-restart-hook\", unknown_field = true",
    );
    assert_error(&source, "invalid TOML or configuration fields");

    let source = EXAMPLE.replace(
        "admin_token = \"test-admin\"",
        "admin_token = { value = \"test-admin\" }",
    );
    assert_error(&source, "invalid TOML or configuration fields");
}

#[test]
fn refuses_to_serve_with_either_example_placeholder() {
    let config = Catalog::parse(EXAMPLE).unwrap();
    config.check_secrets().unwrap();

    let source = EXAMPLE.replace("test-admin", "RUNLEVER_GENERATED_ADMIN_TOKEN");
    let error = Catalog::parse(&source)
        .unwrap()
        .check_secrets()
        .unwrap_err();
    assert!(error.to_string().contains("example admin token"));

    let source = EXAMPLE.replace("test-run-hook", "RUNLEVER_GENERATED_FORGEJO_TOKEN");
    let error = Catalog::parse(&source)
        .unwrap()
        .check_secrets()
        .unwrap_err();
    assert!(error.to_string().contains("example webhook token"));
}

fn assert_error(source: &str, expected: &str) {
    let error = Catalog::parse(source)
        .err()
        .expect("configuration must fail");
    let diagnostic = format!("{error:?}");
    assert!(diagnostic.contains(expected), "{diagnostic}");
    for secret in ["test-admin", "test-run-hook", "test-restart-hook"] {
        assert!(!diagnostic.contains(secret), "{diagnostic}");
    }
}

use super::model::{MAX_NOTIFICATIONS, Model, actions, body};
use handover_core::{
    DeviceId, Notification, NotificationAction, NotificationCommand, NotificationId,
};

fn notification(device_id: &str, local_id: &str) -> Notification {
    Notification {
        id: NotificationId::new(DeviceId::new(device_id), local_id),
        app_name: "Example app".into(),
        title: "A title".into(),
        body: "A body".into(),
        icon_path: None,
        clearable: true,
        actions: vec![NotificationAction {
            id: "open".into(),
            label: "Open".into(),
        }],
        reply_supported: false,
    }
}

#[test]
fn snapshot_baseline_is_idempotent_and_does_not_create_alert_ids() {
    let item = notification("native:phone-a", "42");
    let mut model = Model::default();

    assert!(model.snapshot(vec![item.clone()]).is_empty());
    assert_eq!(model.entries.len(), 1);
    assert_eq!(model.entries[&item.id].desktop_id, None);
    assert!(model.snapshot(vec![item.clone()]).is_empty());
    assert_eq!(model.entries.len(), 1);
    assert_eq!(model.entries[&item.id].desktop_id, None);
}

#[test]
fn notification_ids_with_same_local_id_on_different_devices_stay_distinct() {
    let first = notification("native:phone-a", "42");
    let second = notification("native:phone-b", "42");
    let mut model = Model::default();

    model.snapshot(vec![first.clone(), second.clone()]);

    assert_eq!(model.entries.len(), 2);
    assert!(model.entries.contains_key(&first.id));
    assert!(model.entries.contains_key(&second.id));
}

#[test]
fn action_routing_uses_only_actions_on_the_current_notification() {
    let mut item = notification("native:phone-a", "42");
    item.actions.push(NotificationAction {
        id: "dismiss".into(),
        label: "Dismiss via app action".into(),
    });
    let id = item.id.clone();
    let mut model = Model::default();
    model.snapshot(vec![item.clone()]);
    model.entries.get_mut(&id).unwrap().desktop_id = Some(17);

    assert_eq!(
        model.action(17, "action:open"),
        Some(NotificationCommand::InvokeAction {
            notification_id: id.clone(),
            action_id: "open".into(),
        })
    );
    assert_eq!(
        model.action(17, "action:dismiss"),
        Some(NotificationCommand::InvokeAction {
            notification_id: id.clone(),
            action_id: "dismiss".into(),
        })
    );
    assert_eq!(
        model.action(17, "action:missing"),
        None,
        "stale or invented action IDs must not reach the phone"
    );

    item.actions.retain(|action| action.id == "open");
    model.snapshot(vec![item]);
    assert_eq!(model.action(17, "action:dismiss"), None);
}

#[test]
fn reserved_dismiss_key_does_not_shadow_a_phone_action_named_dismiss() {
    let mut item = notification("native:phone-a", "42");
    item.actions.push(NotificationAction {
        id: "dismiss".into(),
        label: "App action".into(),
    });
    let id = item.id.clone();
    let mut model = Model::default();
    model.snapshot(vec![item]);
    model.entries.get_mut(&id).unwrap().desktop_id = Some(17);

    assert_eq!(
        model.action(17, "dismiss"),
        Some(NotificationCommand::Dismiss {
            notification_id: id.clone(),
        })
    );
    assert_eq!(
        model.action(17, "action:dismiss"),
        Some(NotificationCommand::InvokeAction {
            notification_id: id,
            action_id: "dismiss".into(),
        })
    );
}

#[test]
fn nonclearable_notification_has_no_dismiss_command_or_dismiss_button() {
    let mut item = notification("native:phone-a", "42");
    item.clearable = false;
    let id = item.id.clone();
    let mut model = Model::default();
    model.snapshot(vec![item.clone()]);
    model.entries.get_mut(&id).unwrap().desktop_id = Some(17);

    assert_eq!(model.action(17, "dismiss"), None);
    let rendered = actions(&item);
    assert!(!rendered.iter().any(|value| value == "dismiss"));
    assert!(!rendered.iter().any(|value| value == "Dismiss on phone"));
}

#[test]
fn closed_desktop_popup_does_not_remove_or_dismiss_phone_notification() {
    let item = notification("native:phone-a", "42");
    let id = item.id.clone();
    let mut model = Model::default();
    model.snapshot(vec![item]);
    model.entries.get_mut(&id).unwrap().desktop_id = Some(17);

    model.closed(17);

    assert!(model.entries.contains_key(&id));
    assert_eq!(model.entries[&id].desktop_id, None);
    assert_eq!(model.action(17, "dismiss"), None);
}

#[test]
fn snapshot_removal_closes_only_the_removed_desktop_notification() {
    let kept = notification("native:phone-a", "1");
    let removed = notification("native:phone-a", "2");
    let mut model = Model::default();
    model.snapshot(vec![kept.clone(), removed.clone()]);
    model.entries.get_mut(&kept.id).unwrap().desktop_id = Some(10);
    model.entries.get_mut(&removed.id).unwrap().desktop_id = Some(11);

    assert_eq!(model.snapshot(vec![kept.clone()]), vec![11]);
    assert!(model.entries.contains_key(&kept.id));
    assert!(!model.entries.contains_key(&removed.id));
}

#[test]
fn snapshot_and_dismissal_memory_are_bounded() {
    let notifications: Vec<_> = (0..MAX_NOTIFICATIONS + 5)
        .map(|index| notification("native:phone-a", &index.to_string()))
        .collect();
    let mut model = Model::default();

    model.snapshot(notifications);

    assert_eq!(model.entries.len(), MAX_NOTIFICATIONS);
}

#[test]
fn markup_body_escapes_markup_delimiters_only_when_supported() {
    assert_eq!(body("<b>A & B</b>", true), "&lt;b&gt;A &amp; B&lt;/b&gt;");
    assert_eq!(body("<b>A & B</b>", false), "<b>A & B</b>");
}

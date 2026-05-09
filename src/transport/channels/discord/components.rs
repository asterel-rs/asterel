//! Discord message component builders.
//!
//! Provides type-safe builders for Action Rows, Buttons, Select Menus,
//! and Text Inputs used in interactive Discord messages and modals.

use serde_json::{Value, json};

/// Component type identifiers per Discord API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ComponentType {
    ActionRow = 1,
    Button = 2,
    StringSelect = 3,
    TextInput = 4,
    UserSelect = 5,
    RoleSelect = 6,
    MentionableSelect = 7,
    ChannelSelect = 8,
}

/// Button style identifiers per Discord API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ButtonStyle {
    Primary = 1,
    Secondary = 2,
    Success = 3,
    Danger = 4,
    Link = 5,
}

/// Text input style for modals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TextInputStyle {
    Short = 1,
    Paragraph = 2,
}

/// Build a button component.
#[must_use]
pub fn button(custom_id: &str, label: &str, style: ButtonStyle) -> Value {
    json!({
        "type": ComponentType::Button as u8,
        "custom_id": custom_id,
        "label": label,
        "style": style as u8,
    })
}

/// Build a link button (no `custom_id`, has url).
#[must_use]
pub fn link_button(url: &str, label: &str) -> Value {
    json!({
        "type": ComponentType::Button as u8,
        "url": url,
        "label": label,
        "style": ButtonStyle::Link as u8,
    })
}

/// Build a string select menu component.
#[must_use]
pub fn string_select(
    custom_id: &str,
    placeholder: Option<&str>,
    options: Vec<SelectOption>,
) -> Value {
    let mut component = json!({
        "type": ComponentType::StringSelect as u8,
        "custom_id": custom_id,
        "options": options.into_iter().map(|o| o.to_json()).collect::<Vec<_>>(),
    });
    if let Some(ph) = placeholder {
        component["placeholder"] = json!(ph);
    }
    component
}

/// A single option in a select menu.
#[derive(Debug, Clone)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
}

impl SelectOption {
    /// Create a select option with the given label and value.
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            description: None,
        }
    }

    /// Add an optional description to this select option.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    fn to_json(&self) -> Value {
        let mut opt = json!({
            "label": self.label,
            "value": self.value,
        });
        if let Some(ref desc) = self.description {
            opt["description"] = json!(desc);
        }
        opt
    }
}

/// Build a user select menu component (type 5).
///
/// Discord auto-populates the list with server members.
#[must_use]
pub fn user_select(custom_id: &str, placeholder: Option<&str>) -> Value {
    let mut component = json!({
        "type": ComponentType::UserSelect as u8,
        "custom_id": custom_id,
    });
    if let Some(ph) = placeholder {
        component["placeholder"] = json!(ph);
    }
    component
}

/// Build a role select menu component (type 6).
///
/// Discord auto-populates the list with server roles.
#[must_use]
pub fn role_select(custom_id: &str, placeholder: Option<&str>) -> Value {
    let mut component = json!({
        "type": ComponentType::RoleSelect as u8,
        "custom_id": custom_id,
    });
    if let Some(ph) = placeholder {
        component["placeholder"] = json!(ph);
    }
    component
}

/// Build a mentionable select menu component (type 7).
///
/// Discord auto-populates the list with both users and roles.
#[must_use]
pub fn mentionable_select(custom_id: &str, placeholder: Option<&str>) -> Value {
    let mut component = json!({
        "type": ComponentType::MentionableSelect as u8,
        "custom_id": custom_id,
    });
    if let Some(ph) = placeholder {
        component["placeholder"] = json!(ph);
    }
    component
}

/// Build a channel select menu component (type 8).
///
/// Optionally restrict selectable channel types via `channel_types`
/// (use values from [`DiscordChannelType`] — e.g. `0` for text, `2` for voice).
#[must_use]
pub fn channel_select(
    custom_id: &str,
    placeholder: Option<&str>,
    channel_types: Option<&[u8]>,
) -> Value {
    let mut component = json!({
        "type": ComponentType::ChannelSelect as u8,
        "custom_id": custom_id,
    });
    if let Some(ph) = placeholder {
        component["placeholder"] = json!(ph);
    }
    if let Some(types) = channel_types {
        component["channel_types"] = json!(types);
    }
    component
}

/// Build a text input component (for modals only).
#[must_use]
pub fn text_input(custom_id: &str, label: &str, style: TextInputStyle, required: bool) -> Value {
    json!({
        "type": ComponentType::TextInput as u8,
        "custom_id": custom_id,
        "label": label,
        "style": style as u8,
        "required": required,
    })
}

/// Wrap components in an action row.
#[must_use]
pub fn action_row(components: Vec<Value>) -> Value {
    let components = components.into_iter().collect::<Vec<_>>();
    json!({
        "type": ComponentType::ActionRow as u8,
        "components": components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_builds_correct_json() {
        let btn = button("btn-1", "Click me", ButtonStyle::Primary);
        assert_eq!(btn["type"], 2);
        assert_eq!(btn["custom_id"], "btn-1");
        assert_eq!(btn["label"], "Click me");
        assert_eq!(btn["style"], 1);
    }

    #[test]
    fn link_button_has_url_and_link_style() {
        let btn = link_button("https://example.com", "Visit");
        assert_eq!(btn["type"], 2);
        assert_eq!(btn["url"], "https://example.com");
        assert_eq!(btn["style"], 5);
        assert!(btn.get("custom_id").is_none());
    }

    #[test]
    fn string_select_builds_with_options() {
        let opts = vec![
            SelectOption::new("Option A", "a").with_description("First option"),
            SelectOption::new("Option B", "b"),
        ];
        let sel = string_select("sel-1", Some("Pick one"), opts);
        assert_eq!(sel["type"], 3);
        assert_eq!(sel["custom_id"], "sel-1");
        assert_eq!(sel["placeholder"], "Pick one");
        let options = sel["options"]
            .as_array()
            .expect("string select options should be an array");
        assert_eq!(options.len(), 2);
        assert_eq!(options[0]["label"], "Option A");
        assert_eq!(options[0]["description"], "First option");
        assert!(options[1].get("description").is_none());
    }

    #[test]
    fn text_input_builds_correct_json() {
        let input = text_input("feedback", "Your feedback", TextInputStyle::Paragraph, true);
        assert_eq!(input["type"], 4);
        assert_eq!(input["custom_id"], "feedback");
        assert_eq!(input["style"], 2);
        assert_eq!(input["required"], true);
    }

    #[test]
    fn user_select_with_placeholder() {
        let sel = user_select("u-sel", Some("Pick a user"));
        assert_eq!(sel["type"], 5);
        assert_eq!(sel["custom_id"], "u-sel");
        assert_eq!(sel["placeholder"], "Pick a user");
    }

    #[test]
    fn user_select_without_placeholder() {
        let sel = user_select("u-sel", None);
        assert_eq!(sel["type"], 5);
        assert!(sel.get("placeholder").is_none());
    }

    #[test]
    fn role_select_builds_correct_json() {
        let sel = role_select("r-sel", Some("Pick a role"));
        assert_eq!(sel["type"], 6);
        assert_eq!(sel["custom_id"], "r-sel");
        assert_eq!(sel["placeholder"], "Pick a role");
    }

    #[test]
    fn mentionable_select_builds_correct_json() {
        let sel = mentionable_select("m-sel", None);
        assert_eq!(sel["type"], 7);
        assert_eq!(sel["custom_id"], "m-sel");
        assert!(sel.get("placeholder").is_none());
    }

    #[test]
    fn channel_select_with_types() {
        let sel = channel_select("ch-sel", Some("Pick a channel"), Some(&[0, 5]));
        assert_eq!(sel["type"], 8);
        assert_eq!(sel["custom_id"], "ch-sel");
        assert_eq!(sel["placeholder"], "Pick a channel");
        let types = sel["channel_types"]
            .as_array()
            .expect("channel_types should be an array");
        assert_eq!(types.len(), 2);
        assert_eq!(types[0], 0);
        assert_eq!(types[1], 5);
    }

    #[test]
    fn channel_select_without_types() {
        let sel = channel_select("ch-sel", None, None);
        assert_eq!(sel["type"], 8);
        assert!(sel.get("channel_types").is_none());
    }

    #[test]
    fn action_row_wraps_components() {
        let row = action_row(vec![
            button("a", "A", ButtonStyle::Primary),
            button("b", "B", ButtonStyle::Secondary),
        ]);
        assert_eq!(row["type"], 1);
        let comps = row["components"]
            .as_array()
            .expect("action row components should be an array");
        assert_eq!(comps.len(), 2);
    }
}

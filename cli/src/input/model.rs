/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::{
    commands::Command,
    config::{Config, ProviderConfig},
    harness_actor::HarnessActorHandle,
    input::{InputMode, PromptEditor, picklist::PickList},
    models::{ModelPickerRow, Models, ReasoningEffort, RecentModelDef},
    providers::ProviderDef,
    tui::TuiApp,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl InputMode {
    pub async fn handle_models(
        app: &mut TuiApp,
        key: KeyEvent,
        mut picker: PickList<ModelPickerRow>,
        actor: &HarnessActorHandle,
    ) -> Self {
        app.editor.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => return Self::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Self::Exit;
            }
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.editor.status_clear_input("Add provider");
                let rows = vec![
                    ProviderInfoRow::Name(String::new()),
                    ProviderInfoRow::Url(String::new()),
                    ProviderInfoRow::Kind("openai".to_string()),
                    ProviderInfoRow::Key(String::new()),
                ];
                let picker = PickList::all(rows);
                return Self::AddProvider { picker };
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter => {
                if let Some(busy) = Command::busy_status(app, "changing models") {
                    app.editor.status = busy;
                    return Self::Models { picker };
                }
                let Some(selected_model) = picker.selected_row().and_then(|row| match row {
                    ModelPickerRow::Model(model) | ModelPickerRow::RecentModel(model) => {
                        Some(model)
                    }
                    ModelPickerRow::Header(_) | ModelPickerRow::Separator => None,
                }) else {
                    app.editor.status = "No matching models".to_string();
                    return Self::Models { picker };
                };

                match actor.set_model(selected_model.clone()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.snapshot.model = selected_model.clone();
                        let _ = RecentModelDef::add(selected_model);
                        app.editor.status_clear_input(&format!(
                            "Selected model: {}",
                            selected_model.label()
                        ));
                        return Self::PromptInput;
                    }
                    Err(err) => app.editor.status = err,
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let Some(ModelPickerRow::RecentModel(model)) = picker.selected_row().cloned()
                else {
                    app.editor.status = "Can only delete recent models".to_string();
                    return Self::Models { picker };
                };
                match RecentModelDef::remove(&model) {
                    Ok(updated_recent) => {
                        if let Ok(models) = Models::list(&app.config).await {
                            let recent = RecentModelDef::from_cache(&updated_recent, &models);
                            let rows = Models::filter_rows(&app.editor.input, &models, &recent);
                            let filtered = filter_model_rows(&app.editor.input, &rows);
                            picker.replace_rows(rows, filtered);
                        }
                        app.editor.status = format!("Removed from recent models: {}", model.id);
                    }
                    Err(err) => {
                        app.editor.status = format!("Error removing recent model: {}", err);
                    }
                }
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.editor.status = "Reloading models...".to_string();
                match Models::reload(&app.config).await {
                    Ok(reloaded_models) => {
                        let recent = RecentModelDef::load(&reloaded_models);
                        let rows =
                            Models::filter_rows(&app.editor.input, &reloaded_models, &recent);
                        let filtered = filter_model_rows(&app.editor.input, &rows);
                        let count = reloaded_models.len();
                        picker.replace_rows(rows, filtered);
                        app.editor.status = format!("Reloaded {} models", count);
                    }
                    Err(err) => {
                        app.editor.status = format!("Error reloading models: {}", err);
                    }
                }
            }
            _ if PromptEditor::input_changed_key(key) => {
                picker.replace_filter(filter_model_rows(&app.editor.input, &picker.rows));
            }
            _ => {}
        };

        Self::Models { picker }
    }

    pub async fn handle_provider_add(
        app: &mut TuiApp,
        key: KeyEvent,
        mut picker: PickList<ProviderInfoRow>,
    ) -> Self {
        match key.code {
            KeyCode::Esc => {
                let result = app.editor.build_models(&app.config).await;
                app.editor.status_clear_input("Add provider cancelled");
                return result;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Self::Exit;
            }
            KeyCode::Up => {
                picker.store_input(&app.editor);
                picker.move_up();
                picker.load_input(&mut app.editor);
            }
            KeyCode::Down => {
                picker.store_input(&app.editor);
                picker.move_down();
                picker.load_input(&mut app.editor);
            }
            KeyCode::Enter => {
                picker.store_input(&app.editor);

                let mut name = String::new();
                let mut url = String::new();
                let mut kind = String::new();
                let mut provider_key = String::new();

                for row in &picker.rows {
                    match row {
                        ProviderInfoRow::Name(v) => name = v.clone(),
                        ProviderInfoRow::Url(v) => url = v.clone(),
                        ProviderInfoRow::Kind(v) => kind = v.clone(),
                        ProviderInfoRow::Key(v) => provider_key = v.clone(),
                    }
                }

                let provider = ProviderConfig {
                    name: name.trim().to_string(),
                    url: url.trim().to_string(),
                    kind: kind.trim().to_string(),
                    key: provider_key.trim().to_string(),
                };

                app.editor.status = "Validating provider...".to_string();
                let provider_def = match ProviderDef::new(&provider) {
                    Ok(provider_def) => provider_def,
                    Err(err) => {
                        app.editor.status = format!("Error creating provider: {}", err);
                        return Self::AddProvider { picker };
                    }
                };
                match provider_def.list_models().await {
                    Ok(_) => match app.config.add_provider(provider) {
                        Ok(()) => match Models::reload(&app.config).await {
                            Ok(reloaded_models) => {
                                let recent = RecentModelDef::load(&reloaded_models);
                                let rows = Models::filter_rows("", &reloaded_models, &recent);
                                let filtered = filter_model_rows("", &rows);
                                app.editor.status_clear_input(&format!(
                                    "Added provider; loaded {} models",
                                    reloaded_models.len()
                                ));
                                return Self::Models {
                                    picker: PickList::new(rows, filtered),
                                };
                            }
                            Err(err) => {
                                app.editor.status_clear_input(&format!(
                                    "Added provider, but failed to reload models: {}",
                                    err
                                ));
                                return Self::PromptInput;
                            }
                        },
                        Err(err) => {
                            app.editor.status = format!("Error adding provider: {}", err);
                        }
                    },
                    Err(err) => {
                        app.editor.status = format!("Provider validation failed: {}", err);
                    }
                }
            }
            _ => {
                app.editor.handle_input_cursor(key);
            }
        }

        Self::AddProvider { picker }
    }

    pub async fn handle_reasoning(
        app: &mut TuiApp,
        key: KeyEvent,
        mut picker: PickList<ReasoningEffort>,
        actor: &HarnessActorHandle,
    ) -> Self {
        app.editor.handle_input_cursor(key);
        match key.code {
            KeyCode::Esc => return Self::PromptInput,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Self::Exit;
            }
            KeyCode::Up => {
                picker.move_up();
            }
            KeyCode::Down => {
                picker.move_down();
            }
            KeyCode::Enter => {
                if let Some(busy) = Command::busy_status(app, "changing reasoning") {
                    app.editor.status = busy;
                    return Self::Reasoning { picker };
                }
                let Some(selected) = picker.selected_row() else {
                    return Self::Reasoning { picker };
                };

                // Merge the new reasoning effort into the current model.
                let mut model = app.snapshot.model.clone();
                model.reasoning = if *selected == ReasoningEffort::Default {
                    None
                } else {
                    Some(selected.clone())
                };

                match actor.set_model(model.clone()).await {
                    Ok(snapshot) => {
                        app.save_snapshot(&snapshot);
                        app.snapshot.model = model;
                        let _ = RecentModelDef::add(&app.snapshot.model);
                        app.editor.status_clear_input(&format!(
                            "Reasoning effort = {}",
                            selected.label()
                        ));
                        return Self::PromptInput;
                    }
                    Err(err) => app.editor.status = err,
                }
            }
            _ => {}
        }

        Self::Reasoning { picker }
    }
}

impl PromptEditor {
    pub async fn build_models(&mut self, config: &Config) -> InputMode {
        match Models::list(config).await {
            Ok(models) => {
                let recent = RecentModelDef::load(&models);
                let rows = Models::filter_rows("", &models, &recent);
                let filtered = filter_model_rows("", &rows);
                self.status = format!("Loaded {} models", models.len());
                InputMode::Models {
                    picker: PickList::new(rows, filtered),
                }
            }
            Err(err) => {
                self.status = format!("Error loading models: {}", err);
                InputMode::PromptInput
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderInfoRow {
    Name(String),
    Url(String),
    Kind(String),
    Key(String),
}

impl ProviderInfoRow {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Name(_) => "Name",
            Self::Url(_) => "URL",
            Self::Kind(_) => "Kind",
            Self::Key(_) => "Key",
        }
    }

    pub fn value(&self) -> &str {
        match self {
            Self::Name(v) | Self::Url(v) | Self::Kind(v) | Self::Key(v) => v,
        }
    }

    pub fn set_value(&mut self, new_value: String) {
        match self {
            Self::Name(v) => *v = new_value,
            Self::Url(v) => *v = new_value,
            Self::Kind(v) => *v = new_value,
            Self::Key(v) => *v = new_value,
        }
    }

    pub fn masked_value(&self) -> String {
        match self {
            Self::Key(v) if !v.is_empty() => "*".repeat(v.len()),
            _ => self.value().to_string(),
        }
    }
}

impl PickList<ProviderInfoRow> {
    pub fn store_input(&mut self, editor: &PromptEditor) {
        if let Some(row) = self.selected_row_mut() {
            row.set_value(editor.input.clone());
        }
    }

    pub fn load_input(&mut self, editor: &mut PromptEditor) {
        if let Some(row) = self.selected_row() {
            editor.input = row.value().to_string();
            editor.cursor = editor.input.len();
        }
    }
}

pub fn filter_model_rows(input: &str, rows: &[ModelPickerRow]) -> Vec<usize> {
    let query = input.trim().to_lowercase();
    rows.iter()
        .enumerate()
        .filter_map(|(index, row)| match row {
            ModelPickerRow::Model(model) | ModelPickerRow::RecentModel(model)
                if query.is_empty() || model.id.to_lowercase().contains(&query) =>
            {
                Some(index)
            }
            ModelPickerRow::Header(_)
            | ModelPickerRow::Separator
            | ModelPickerRow::Model(_)
            | ModelPickerRow::RecentModel(_) => None,
        })
        .collect()
}

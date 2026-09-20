//! Automatic restore for a monitored process whose save location is missing.
//!
//! This is the only process trigger that writes live save data, so every outcome
//! that needs player attention is reported with its own wording instead of the
//! generic quick-action notification.

use log::{info, warn};
use tauri::{AppHandle, Manager};

use rgsm_core::backup::Game;
use rgsm_core::config::{QuickActionSoundPreferences, get_config};
use rgsm_core::hooks::HookSource;
use rgsm_core::preclude::show_notification;
use rgsm_core::services::{AutoRestoreDecision, ServiceContext};
use rust_i18n::t;

use crate::sound::{QuickActionSoundEffect, play_quick_action_sound};

use super::{QuickActionOperation, QuickActionStatus, QuickActionType, emit_quick_action_event};

pub async fn perform_auto_restore(app: &AppHandle, game: &Game) {
    let service = ServiceContext::new(app.state::<crate::hooks::HookPipelineState>().snapshot());
    let decision = service
        .restore_missing_save_path(game, HookSource::ProcessMonitorAutoRestore, None)
        .await;

    match decision {
        Ok(AutoRestoreDecision::Restored { date }) => {
            info!(
                target: "rgsm::quick_action::auto_restore",
                "Restored '{}' to Snapshot {date}: its save location was missing",
                game.name
            );
            report(
                app,
                game,
                QuickActionStatus::Success,
                t!("backend.auto_restore.restored"),
                t!(
                    "backend.auto_restore.restored_detail",
                    game = game.name.as_str(),
                    date = date.as_str()
                ),
            );
        }
        Ok(AutoRestoreDecision::NoBackupAvailable) => {
            warn!(
                target: "rgsm::quick_action::auto_restore",
                "Cannot restore '{}': no Snapshot exists",
                game.name
            );
            report(
                app,
                game,
                QuickActionStatus::Failure,
                t!("backend.auto_restore.failed"),
                t!("backend.auto_restore.no_backup_detail", game = game.name.as_str()),
            );
        }
        Ok(AutoRestoreDecision::SkippedNoLocalArchive) => {
            warn!(
                target: "rgsm::quick_action::auto_restore",
                "Cannot restore '{}': no Local Archive on this device",
                game.name
            );
            report(
                app,
                game,
                QuickActionStatus::Failure,
                t!("backend.auto_restore.failed"),
                t!("backend.auto_restore.archive_missing_detail", game = game.name.as_str()),
            );
        }
        Ok(decision) => {
            info!(
                target: "rgsm::quick_action::auto_restore",
                "Skipped auto-restore for '{}': {decision:?}",
                game.name
            );
        }
        Err(err) => {
            warn!(
                target: "rgsm::quick_action::auto_restore",
                "Auto-restore failed for '{}': {err:?}",
                game.name
            );
            report(
                app,
                game,
                QuickActionStatus::Failure,
                t!("backend.auto_restore.failed"),
                format!(
                    "{}\n{err}",
                    t!("backend.auto_restore.failed_detail", game = game.name.as_str())
                ),
            );
        }
    }
}

fn report(
    app: &AppHandle,
    game: &Game,
    status: QuickActionStatus,
    title: impl AsRef<str>,
    body: impl AsRef<str>,
) {
    let config = match get_config() {
        Ok(config) => config,
        Err(err) => {
            warn!(
                target: "rgsm::quick_action::auto_restore",
                "Failed to load config while reporting auto-restore: {err:?}"
            );
            return;
        }
    };

    let quick_settings = &config.quick_action;
    if quick_settings.enable_notification {
        show_notification(title, body);
    }
    let sound_preferences = QuickActionSoundPreferences::from(quick_settings);
    let effect = match status {
        QuickActionStatus::Success => QuickActionSoundEffect::Success,
        _ => QuickActionSoundEffect::Failure,
    };
    play_quick_action_sound(app, sound_preferences, effect);
    emit_quick_action_event(
        app,
        QuickActionType::ProcessStart,
        QuickActionOperation::Apply,
        status,
        Some(game.name.clone()),
    );
}

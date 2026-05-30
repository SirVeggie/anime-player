import { useEffect, useState } from "react";
import type {
  AnilistAuthState,
  Category,
  LibraryState,
  LocalDataStats,
  RegexRule,
  RegexRuleInput,
  RootFolder,
} from "../types";
import { CustomCheckbox } from "./CustomCheckbox";
import { ViewHeader } from "./ViewHeader";

const EMPTY_RULE: RegexRuleInput = {
  name: "",
  detection_regex: "",
  title_regex: "",
  enabled: true,
  priority: 0,
};

export function SettingsScreen(props: {
  library: LibraryState;
  busy: boolean;
  rootInput: string;
  newCategoryName: string;
  newRuleEditorKey: number;
  anilistAuth: AnilistAuthState | null;
  localDataStats: LocalDataStats | null;
  onBack: () => void;
  onRootInput: (value: string) => void;
  onPickFolder: () => void;
  onAddRoot: () => void;
  onRemoveRoot: (root: RootFolder) => void;
  onRescan: () => void;
  onNewCategoryName: (value: string) => void;
  onCreateCategory: () => void;
  onDeleteCategory: (category: Category) => void;
  onSetDefaultCategory: (category: Category) => void;
  onReorderCategory: (category: Category, direction: "up" | "down") => void;
  onCreateRule: (input: RegexRuleInput) => void;
  onUpdateRule: (id: number, input: RegexRuleInput) => void;
  onDeleteRule: (rule: RegexRule) => void;
  onSaveAnilistClientId: (clientId: string) => void;
  onLoginAnilist: () => void;
  onLogoutAnilist: () => void;
  onPreferAnilistDisplayTitle: (enabled: boolean) => void;
  onCleanLocalData: () => void;
}) {
  const {
    library,
    busy,
    rootInput,
    newCategoryName,
    newRuleEditorKey,
    anilistAuth,
    localDataStats,
    onBack,
    onRootInput,
    onPickFolder,
    onAddRoot,
    onRemoveRoot,
    onRescan,
    onNewCategoryName,
    onCreateCategory,
    onDeleteCategory,
    onSetDefaultCategory,
    onReorderCategory,
    onCreateRule,
    onUpdateRule,
    onDeleteRule,
    onSaveAnilistClientId,
    onLoginAnilist,
    onLogoutAnilist,
    onPreferAnilistDisplayTitle,
    onCleanLocalData,
  } = props;
  const [anilistClientDraft, setAnilistClientDraft] = useState(anilistAuth?.client_id ?? "");

  useEffect(() => {
    setAnilistClientDraft(anilistAuth?.client_id ?? "");
  }, [anilistAuth?.client_id]);

  return (
    <>
      <ViewHeader title="Settings" subtitle={`Portable database: ${library.db_path}`} onBack={onBack} />

      <section className="panel">
        <div className="panel-heading">
          <h2>AniList</h2>
          <span className="muted">
            {anilistAuth?.authenticated ? `Logged in as ${anilistAuth.viewer_name ?? "AniList user"}` : "Not logged in"}
          </span>
        </div>
        <form
          className="form-row"
          onSubmit={(e) => {
            e.preventDefault();
            onSaveAnilistClientId(anilistClientDraft);
          }}
        >
          <input
            type="text"
            value={anilistClientDraft}
            onChange={(e) => setAnilistClientDraft(e.currentTarget.value)}
            placeholder="AniList OAuth client ID..."
            spellCheck={false}
          />
          <button type="submit" disabled={busy}>
            Save
          </button>
          <button type="button" onClick={onLoginAnilist} disabled={busy || !anilistClientDraft.trim()}>
            Login with AniList
          </button>
          {anilistAuth?.authenticated ? (
            <button type="button" onClick={onLogoutAnilist} disabled={busy}>
              Logout
            </button>
          ) : null}
        </form>
        <p className="muted">
          The default client ID works for normal use, so you can press Login with AniList without changing it.
          If you create your own AniList app, set its redirect URL to <code>anime-player://anilist-auth</code>.
        </p>
        <div className="settings-item settings-item--stacked">
          <CustomCheckbox
            checked={library.prefer_anilist_display_title}
            disabled={busy}
            onChange={onPreferAnilistDisplayTitle}
            label="Use AniList title in the library when linked"
          />
          <p className="muted">
            Linked titles show the AniList name in grids, search, and episode headers. Unlinked titles always use the
            detected name from your files.
          </p>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h2>Local Data</h2>
          <span className="muted">{localDataStats ? formatBytes(localDataStats.total_bytes) : "Calculating..."}</span>
        </div>
        <div className="local-data-summary">
          <div className="local-data-stat">
            <span>Database</span>
            <strong>{localDataStats ? formatBytes(localDataStats.database_bytes) : "-"}</strong>
          </div>
          <div className="local-data-stat">
            <span>Saved thumbnails</span>
            <strong>{localDataStats ? formatBytes(localDataStats.thumbnails_bytes) : "-"}</strong>
          </div>
          <div className="local-data-stat">
            <span>Scrub sprites</span>
            <strong>{localDataStats ? formatBytes(localDataStats.scrub_sprites_bytes) : "-"}</strong>
          </div>
        </div>
        <p className="muted">
          Rescans keep temporarily missing or unmatched episodes so links, categories, and progress are not lost.
          Use cleanup after your rules and folders are correct.
        </p>
        <div className="settings-actions">
          <button type="button" onClick={onCleanLocalData} disabled={busy}>
            Clean local data
          </button>
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h2>Root Folders</h2>
          <button type="button" onClick={onRescan} disabled={busy || library.root_folders.length === 0}>
            Rescan
          </button>
        </div>
        <form
          className="form-row"
          onSubmit={(e) => {
            e.preventDefault();
            onAddRoot();
          }}
        >
          <input
            type="text"
            value={rootInput}
            onChange={(e) => onRootInput(e.currentTarget.value)}
            placeholder="Paste a root folder path..."
            spellCheck={false}
          />
          <button type="button" onClick={onPickFolder} disabled={busy}>
            Browse
          </button>
          <button type="submit" disabled={busy}>
            Add
          </button>
        </form>
        <div className="settings-list">
          {library.root_folders.map((root) => (
            <div className="settings-item" key={root.id}>
              <span title={root.path}>{root.path}</span>
              <button type="button" onClick={() => onRemoveRoot(root)} disabled={busy}>
                Remove
              </button>
            </div>
          ))}
          {library.root_folders.length === 0 ? <p className="muted">No root folders configured.</p> : null}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h2>Categories</h2>
        </div>
        <form
          className="form-row"
          onSubmit={(e) => {
            e.preventDefault();
            onCreateCategory();
          }}
        >
          <input
            type="text"
            value={newCategoryName}
            onChange={(e) => onNewCategoryName(e.currentTarget.value)}
            placeholder="New category name..."
          />
          <button type="submit" disabled={busy || !newCategoryName.trim()}>
            Add
          </button>
        </form>
        <div className="settings-list">
          {library.categories.map((category, index) => (
            <div className="settings-item" key={category.id}>
              <span>
                {category.name} {category.is_default ? <span className="pill">Default</span> : null}
              </span>
              <div className="settings-actions">
                <button
                  type="button"
                  onClick={() => onSetDefaultCategory(category)}
                  disabled={busy || category.is_default}
                >
                  Make default
                </button>
                <button
                  type="button"
                  onClick={() => onDeleteCategory(category)}
                  disabled={busy || category.is_default}
                >
                  Delete
                </button>
                <div className="settings-reorder-controls" aria-label="Reorder category">
                  <button
                    type="button"
                    className="settings-reorder-button"
                    onClick={() => onReorderCategory(category, "up")}
                    disabled={busy || index === 0}
                    aria-label={`Move ${category.name} up`}
                  >
                    <span aria-hidden>▴</span>
                  </button>
                  <button
                    type="button"
                    className="settings-reorder-button"
                    onClick={() => onReorderCategory(category, "down")}
                    disabled={busy || index === library.categories.length - 1}
                    aria-label={`Move ${category.name} down`}
                  >
                    <span aria-hidden>▾</span>
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h2>Detection Rules</h2>
          <span className="muted">
            {library.regex_rules.length} configured · higher priority matches first
          </span>
        </div>
        <div className="settings-list">
          {library.regex_rules.map((rule) => (
            <RuleEditor
              key={rule.id}
              title={rule.name}
              busy={busy}
              initial={ruleToInput(rule)}
              submitLabel="Save"
              onSubmit={(input) => onUpdateRule(rule.id, input)}
              onDelete={() => onDeleteRule(rule)}
            />
          ))}
        </div>
        <RuleEditor
          key={newRuleEditorKey}
          title="New rule"
          busy={busy}
          initial={EMPTY_RULE}
          submitLabel="Add rule"
          onSubmit={onCreateRule}
        />
      </section>
    </>
  );
}

function RuleEditor(props: {
  title: string;
  busy: boolean;
  initial: RegexRuleInput;
  submitLabel: string;
  onSubmit: (input: RegexRuleInput) => void;
  onDelete?: () => void;
}) {
  const { title, busy, initial, submitLabel, onSubmit, onDelete } = props;
  const [draft, setDraft] = useState(initial);

  return (
    <form
      className="rule-editor"
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit(draft);
      }}
    >
      <div className="rule-editor-heading">
        <strong>{title}</strong>
        <CustomCheckbox
          checked={draft.enabled}
          onChange={(enabled) => setDraft((current) => ({ ...current, enabled }))}
          label="Enabled"
        />
      </div>
      <div className="form-grid">
        <label>
          <span>Name</span>
          <input
            type="text"
            value={draft.name}
            onChange={(e) => {
              const name = e.currentTarget.value;
              setDraft((current) => ({ ...current, name }));
            }}
          />
        </label>
        <label>
          <span>Priority</span>
          <div className="score-stepper">
            <input
              type="number"
              value={draft.priority}
              disabled={busy}
              onChange={(e) => {
                const priority = Number(e.currentTarget.value) || 0;
                setDraft((current) => ({ ...current, priority }));
              }}
            />
            <div className="score-stepper-buttons">
              <button
                type="button"
                className="score-stepper-button score-stepper-button--up"
                aria-label="Increase priority"
                disabled={busy}
                onClick={() =>
                  setDraft((current) => ({
                    ...current,
                    priority: current.priority + 1,
                  }))
                }
              />
              <button
                type="button"
                className="score-stepper-button score-stepper-button--down"
                aria-label="Decrease priority"
                disabled={busy}
                onClick={() =>
                  setDraft((current) => ({
                    ...current,
                    priority: current.priority - 1,
                  }))
                }
              />
            </div>
          </div>
        </label>
      </div>
      <div className="rule-editor-regex-row">
        <label>
          <span>Detection regex</span>
          <input
            type="text"
            value={draft.detection_regex}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => {
              const detection_regex = e.currentTarget.value;
              setDraft((current) => ({ ...current, detection_regex }));
            }}
          />
        </label>
        <label>
          <span>Title regex</span>
          <input
            type="text"
            value={draft.title_regex}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => {
              const title_regex = e.currentTarget.value;
              setDraft((current) => ({ ...current, title_regex }));
            }}
          />
        </label>
      </div>
      <div className="settings-actions">
        <button type="submit" disabled={busy}>
          {submitLabel}
        </button>
        {onDelete ? (
          <button type="button" onClick={onDelete} disabled={busy}>
            Delete
          </button>
        ) : null}
      </div>
    </form>
  );
}

function ruleToInput(rule: RegexRule): RegexRuleInput {
  return {
    name: rule.name,
    detection_regex: rule.detection_regex,
    title_regex: rule.title_regex,
    enabled: rule.enabled,
    priority: rule.priority,
  };
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  return `${value >= 10 ? value.toFixed(1) : value.toFixed(2)} ${units[unitIndex]}`;
}

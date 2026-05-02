import { useEffect, useState } from "react";
import type { Category, LibraryState, RegexRule, RegexRuleInput, RootFolder } from "../types";
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
  onCreateRule: (input: RegexRuleInput) => void;
  onUpdateRule: (id: number, input: RegexRuleInput) => void;
  onDeleteRule: (rule: RegexRule) => void;
}) {
  const {
    library,
    busy,
    rootInput,
    newCategoryName,
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
    onCreateRule,
    onUpdateRule,
    onDeleteRule,
  } = props;

  return (
    <>
      <ViewHeader title="Settings" subtitle={`Portable database: ${library.db_path}`} onBack={onBack} />

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
          {library.categories.map((category) => (
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
              </div>
            </div>
          ))}
        </div>
      </section>

      <section className="panel">
        <div className="panel-heading">
          <h2>Detection Rules</h2>
          <span className="muted">{library.regex_rules.length} configured</span>
        </div>
        <RuleEditor
          title="Add rule"
          busy={busy}
          initial={EMPTY_RULE}
          submitLabel="Add rule"
          onSubmit={onCreateRule}
        />
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

  useEffect(() => {
    setDraft(initial);
  }, [initial]);

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
        <label>
          <input
            type="checkbox"
            checked={draft.enabled}
            onChange={(e) => {
              // Read the event field synchronously: React's functional updater
              // can run twice under StrictMode, and by the second pass `e.currentTarget`
              // has been nulled, throwing inside RuleEditor and unmounting the whole tree.
              const enabled = e.currentTarget.checked;
              setDraft((current) => ({ ...current, enabled }));
            }}
          />
          Enabled
        </label>
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
          <input
            type="number"
            value={draft.priority}
            onChange={(e) => {
              const priority = Number(e.currentTarget.value) || 0;
              setDraft((current) => ({ ...current, priority }));
            }}
          />
        </label>
      </div>
      <label className="stacked-field">
        <span>Detection regex</span>
        <textarea
          value={draft.detection_regex}
          onChange={(e) => {
            const detection_regex = e.currentTarget.value;
            setDraft((current) => ({ ...current, detection_regex }));
          }}
          rows={2}
        />
      </label>
      <label className="stacked-field">
        <span>Title regex</span>
        <textarea
          value={draft.title_regex}
          onChange={(e) => {
            const title_regex = e.currentTarget.value;
            setDraft((current) => ({ ...current, title_regex }));
          }}
          rows={2}
        />
      </label>
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

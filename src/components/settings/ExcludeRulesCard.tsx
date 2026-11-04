import { IconPlus, IconX } from "@tabler/icons-preact";
import { useState } from "preact/hooks";

interface ExcludeRulesCardProps {
  patterns: string[] | null;
  onChange: (patterns: string[]) => Promise<void>;
}

/** Settings → Excluded from sync: rules for paths that are never synced. */
export function ExcludeRulesCard({ patterns, onChange }: ExcludeRulesCardProps) {
  const [draft, setDraft] = useState("");
  const trimmedDraft = draft.trim();

  const addRule = () => {
    if (!patterns || !trimmedDraft || patterns.includes(trimmedDraft)) {
      return;
    }
    void onChange([...patterns, trimmedDraft]).then(() => setDraft(""));
  };

  return (
    <article class="card">
      <h3>Excluded from sync</h3>
      <p class="setting-row-description">
        Excluded items are never uploaded, downloaded or deleted, on either side. A rule with a <code>/</code> is a path
        from the top of the sync folder, such as <code>Documents/Chromium</code>; a rule without one matches a name
        anywhere, such as <code>node_modules</code> or <code>*.tmp</code>.
      </p>
      {patterns === null ? (
        <p class="setting-row-description">Exclude rules could not be loaded.</p>
      ) : (
        <div class="settings-list">
          {patterns.length === 0 ? (
            <p class="setting-row-description">No rules yet: everything in your sync folders is synced.</p>
          ) : (
            <ul class="exclude-rule-list">
              {patterns.map((pattern) => (
                <li key={pattern} class="exclude-rule">
                  <code>{pattern}</code>
                  <button
                    type="button"
                    class="ghost header-icon-btn"
                    aria-label={`Remove rule ${pattern}`}
                    title="Remove rule"
                    onClick={() => void onChange(patterns.filter((existing) => existing !== pattern))}
                  >
                    <IconX size={15} />
                  </button>
                </li>
              ))}
            </ul>
          )}
          <form
            class="setting-row"
            onSubmit={(event) => {
              event.preventDefault();
              addRule();
            }}
          >
            <div class="setting-row-copy">
              <label class="setting-row-label" htmlFor="exclude-rule-input">
                Add a rule
              </label>
            </div>
            <div class="setting-row-controls">
              <input
                id="exclude-rule-input"
                value={draft}
                placeholder="Documents/Chromium"
                onInput={(event) => setDraft(event.currentTarget.value)}
              />
              <button type="submit" class="ghost" disabled={!trimmedDraft || patterns.includes(trimmedDraft)}>
                <IconPlus size={15} />
                Add
              </button>
            </div>
          </form>
        </div>
      )}
    </article>
  );
}

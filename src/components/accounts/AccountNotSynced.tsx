import { IconBan, IconFolder, IconNotebook, IconShieldX, IconUsers } from "@tabler/icons-preact";
import type { NotSyncedItem } from "../../types/somedrive";
import { splitSyncPath } from "./syncFormatting";

interface AccountNotSyncedProps {
  items: NotSyncedItem[];
  onOpenItemFolder: (relativePath: string) => Promise<void>;
}

function ReasonIcon({ item }: { item: NotSyncedItem }) {
  switch (item.reasonCode) {
    case "onenote_notebook":
      return <IconNotebook size={22} />;
    case "shared_with_you":
      return <IconUsers size={22} />;
    case "malware_detected":
      return <IconShieldX size={22} />;
    case "excluded_by_rule":
      return item.isFolder ? <IconFolder size={22} /> : <IconBan size={22} />;
    default:
      return <IconBan size={22} />;
  }
}

function containedText(item: NotSyncedItem): string | null {
  if (item.contained === 0) {
    return null;
  }
  const noun = item.reasonCode === "onenote_notebook" ? "section" : "item";
  return `${item.contained} ${noun}${item.contained === 1 ? "" : "s"} inside`;
}

/** Items SomeDrive skips by design, one line each, with the reason. */
export function AccountNotSynced({ items, onOpenItemFolder }: AccountNotSyncedProps) {
  if (items.length === 0) {
    return <p class="sync-activity-empty">Everything in OneDrive is synced to this device.</p>;
  }
  return (
    <div class="sync-problems">
      <p class="sync-problems-note">
        These are skipped on purpose and need nothing from you. They stay in OneDrive as they are.
      </p>
      <ul class="sync-problem-list">
        {items.map((item) => {
          const { name, folder } = splitSyncPath(item.path);
          const inside = containedText(item);
          return (
            <li key={`${item.reasonCode}:${item.path}`} class="sync-problem is-not-synced">
              <div class="sync-problem-head">
                <span class="sync-problem-icon">
                  <ReasonIcon item={item} />
                </span>
                <button type="button" class="sync-problem-name" onClick={() => void onOpenItemFolder(item.path)}>
                  <span class="sync-file-name">{name}</span>
                  <span class="sync-file-folder">
                    {folder || "OneDrive"}
                    {inside ? ` · ${inside}` : ""}
                  </span>
                </button>
              </div>
              <p class="sync-problem-detail">{item.reason}</p>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

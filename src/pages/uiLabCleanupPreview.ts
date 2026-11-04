import type { CleanupActions, CleanupView } from "../types/somedrive";

const GB = 1024 * 1024 * 1024;
const MB = 1024 * 1024;

/** Sample Clean up data for the UI Lab preview. */
export const previewCleanupView: CleanupView = {
  duplicateBytes: 7.9 * GB,
  hidden: ["Pictures/Albums/Collection of Red"],
  duplicates: [
    {
      id: "partial:8942:7300000000:Games/Backup/20250223",
      kind: "partial",
      path: "Games/Backup/20250223",
      deletePaths: ["Games/Backup/20250223/ROM Library", "Games/Backup/20250223/SYSTEM/CHEAT"],
      fileCount: 8942,
      bytes: 7.3 * GB,
      folderFileCount: 8945,
      keptIn: [{ path: "Games/Backup/20250809", fileCount: 8942 }],
      keptInMore: 0,
      pairs: [
        {
          path: "Games/Backup/20250223/ROM Library/Pokemon - Emerald Version (U).gba",
          keptPath: "Games/Backup/20250809/ROM Library/Pokemon - Emerald Version (U).gba",
          size: 16 * MB,
        },
        {
          path: "Games/Backup/20250223/ROM Library/Mario Golf - Advance Tour (U).gba",
          keptPath: "Games/Backup/20250809/ROM Library/Mario Golf - Advance Tour (U).gba",
          size: 16 * MB,
        },
      ],
    },
    {
      id: "folder:201:622570334:Pictures/duplicates",
      kind: "folder",
      path: "Pictures/duplicates",
      deletePaths: ["Pictures/duplicates"],
      fileCount: 201,
      bytes: 594 * MB,
      folderFileCount: 201,
      keptIn: [
        { path: "Pictures/Archive/2017/10", fileCount: 189 },
        { path: "Pictures/Archive/2018/03", fileCount: 9 },
      ],
      keptInMore: 1,
      pairs: [
        {
          path: "Pictures/duplicates/Phone/IMG_2739.MOV",
          keptPath: "Pictures/Archive/2017/10/IMG_2739.MOV",
          size: 52 * MB,
        },
      ],
    },
    {
      id: "files:2:100:Pictures/Old",
      kind: "files",
      path: "Pictures/Old",
      deletePaths: ["Pictures/Old/invitations.jpg", "Pictures/Old/polaroid.jpg"],
      fileCount: 2,
      bytes: 1.2 * MB,
      folderFileCount: 23,
      keptIn: [{ path: "Pictures/Albums/Wedding", fileCount: 2 }],
      keptInMore: 0,
      pairs: [],
    },
    {
      id: "files:1:2000:Docs",
      kind: "files",
      path: "Docs",
      deletePaths: ["Docs/notes.txt"],
      fileCount: 1,
      bytes: 2000,
      folderFileCount: 40,
      keptIn: [{ path: "Docs/Archive", fileCount: 1 }],
      keptInMore: 0,
      pairs: [],
    },
  ],
  largeFiles: [
    { path: "Docs - Tayla/manjaro-kde-18.0.4-stable-x86_64.iso", size: 2 * GB, modified: 1_560_000_000, copies: [] },
    {
      path: "Pictures/Archive/2022/12/PXL_20221224_211948351.mp4",
      size: 229 * MB,
      modified: 1_671_900_000,
      copies: ["Pictures/Albums/Collection of Red/PXL_20221224_211948351.mp4"],
    },
  ],
};

/** Preview actions: nothing is deleted; a delete reports what it would do. */
export const previewCleanupActions: CleanupActions = {
  list: async () => previewCleanupView,
  deleteDuplicates: async () => Promise.reject("UI Lab preview: nothing was deleted."),
  deleteLargeFile: async () => Promise.reject("UI Lab preview: nothing was deleted."),
  hide: async () => undefined,
  unhide: async () => undefined,
};

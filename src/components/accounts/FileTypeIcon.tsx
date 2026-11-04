import {
  IconCode,
  IconFile,
  IconFileDescription,
  IconFileMusic,
  IconFileSpreadsheet,
  IconFileTypePdf,
  IconFileWord,
  IconFileZip,
  IconPhoto,
  IconVideo,
} from "@tabler/icons-preact";

const EXTENSION_GROUPS: Array<{ extensions: string[]; Icon: typeof IconFile }> = [
  { extensions: ["jpg", "jpeg", "png", "gif", "webp", "bmp", "svg", "heic", "tif", "tiff", "ico"], Icon: IconPhoto },
  { extensions: ["mp4", "mkv", "mov", "avi", "wmv", "webm", "m4v", "flv"], Icon: IconVideo },
  { extensions: ["mp3", "wav", "flac", "m4a", "aac", "ogg", "wma"], Icon: IconFileMusic },
  { extensions: ["doc", "docx", "odt", "rtf", "txt", "md"], Icon: IconFileWord },
  { extensions: ["xls", "xlsx", "csv", "ods", "tsv"], Icon: IconFileSpreadsheet },
  { extensions: ["pdf"], Icon: IconFileTypePdf },
  { extensions: ["zip", "rar", "7z", "tar", "gz", "bz2", "xz"], Icon: IconFileZip },
  {
    extensions: ["json", "yaml", "yml", "toml", "xml", "ini", "conf", "cfg", "env", "ts", "tsx", "js", "jsx", "rs", "py", "sh"],
    Icon: IconCode,
  },
  { extensions: ["ppt", "pptx", "odp"], Icon: IconFileDescription },
];

function extensionFromPath(path: string): string {
  const filename = path.split("/").pop() ?? "";
  const dotIndex = filename.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === filename.length - 1) {
    return "";
  }
  return filename.slice(dotIndex + 1).toLowerCase();
}

interface FileTypeIconProps {
  path: string;
  size: number;
}

export function FileTypeIcon({ path, size }: FileTypeIconProps) {
  const extension = extensionFromPath(path);
  const Icon = EXTENSION_GROUPS.find((group) => group.extensions.includes(extension))?.Icon ?? IconFile;
  return <Icon size={size} stroke={1.6} />;
}

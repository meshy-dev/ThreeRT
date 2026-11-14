/** Every downloadable artifact, including the Windows portable build and checksum file. */
export function releaseArtifacts(latest: Record<string, any>) {
  const entries = [
    { key: "macos", name: "macOS", required: true, entry: latest.macos },
    { key: "linux", name: "Linux", required: false, entry: latest.linux },
    { key: "android", name: "Android", required: false, entry: latest.android },
    { key: "windows", name: "Windows", required: false, entry: latest.windows },
  ];
  if (latest.windows) for (const part of ["portable", "checksums"])
    entries.push({ key: `windows.${part}`, name: `Windows ${part}`, required: true, entry: latest.windows[part] });
  return entries;
}

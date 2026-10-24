declare module "virtual:settingsLibrary" {
  export interface SettingsLibraryEntry {
    id: string;
    name: string;
    description: string;
    settings: import("./composables/settings").SettingType[];
  }
  const presets: SettingsLibraryEntry[];
  export default presets;
}

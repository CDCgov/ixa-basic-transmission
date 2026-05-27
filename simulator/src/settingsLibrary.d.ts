declare module "virtual:settingsLibrary" {
  import type { SettingType } from "./composables/settings";

  export interface SettingsLibraryEntry {
    id: string;
    name: string;
    description: string;
    settings: SettingType[];
  }
  const presets: SettingsLibraryEntry[];
  export default presets;
}

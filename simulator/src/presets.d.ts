declare module "virtual:presets" {
  export interface Preset {
    id: string;
    name: string;
    description: string;
    parameters: Record<string, unknown>;
  }
  const presets: Preset[];
  export default presets;
}

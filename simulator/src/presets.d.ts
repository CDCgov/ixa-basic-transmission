declare module "virtual:presets" {
  export interface Preset {
    id: string;
    name: string;
    description: string;
    parameters: Record<string, number>;
  }
  const presets: Preset[];
  export default presets;
}

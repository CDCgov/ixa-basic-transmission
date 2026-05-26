declare module "virtual:rateLibrary" {
  // One curve per entry: an array of [τ, rate] anchor points, sorted by τ.
  const curves: [number, number][][];
  export default curves;
}

import { z } from 'zod';

export const GunParametersSchema = z.object({
  filament_radius_mm: z.number(),
  filament_thickness_mm: z.number(),
  filament_z_mm: z.number(),
  filament_voltage_v: z.number(),
  wehnelt_outer_radius_mm: z.number(),
  wehnelt_inner_radius_mm: z.number(),
  wehnelt_z_mm: z.number(),
  wehnelt_height_mm: z.number(),
  wehnelt_cap_thickness_mm: z.number(),
  wehnelt_aperture_radius_mm: z.number(),
  wehnelt_bias_v: z.number(),
  anode_z_mm: z.number(),
  anode_thickness_mm: z.number(),
  anode_outer_radius_mm: z.number(),
  anode_aperture_radius_mm: z.number(),
  anode_voltage_v: z.number(),
  h_scale: z.number(),
  wehnelt_enabled: z.boolean(),
  anode_enabled: z.boolean(),
});

export type GunParameters = z.infer<typeof GunParametersSchema>;

// Mirrors GunParameters::default() in crates/wasm-api/src/lib.rs.
export const DEFAULT_GUN_PARAMETERS: GunParameters = {
  filament_radius_mm: 0.075,
  filament_thickness_mm: 0.075,
  filament_z_mm: 0,
  filament_voltage_v: -10_000.0,

  wehnelt_outer_radius_mm: 7.5,
  wehnelt_inner_radius_mm: 5.5,
  wehnelt_z_mm: 4.5,
  wehnelt_height_mm: 10.0,
  wehnelt_cap_thickness_mm: 1.5,
  wehnelt_aperture_radius_mm: 1.5,
  wehnelt_bias_v: -100.0,

  anode_z_mm: -10.0,
  anode_thickness_mm: 2.0,
  anode_outer_radius_mm: 8.0,
  anode_aperture_radius_mm: 2,
  anode_voltage_v: 0.0,

  h_scale: 1.0,
  wehnelt_enabled: true,
  anode_enabled: true,
};

export const GunSolutionSchema = z.object({
  n_r: z.number().int(),
  n_z: z.number().int(),
  h_m: z.number(),
  z_lo_m: z.number(),
  potential_v: z.instanceof(Float64Array),
  e_r_v_per_m: z.instanceof(Float64Array),
  e_z_v_per_m: z.instanceof(Float64Array),
  mask: z.instanceof(Uint8Array),
});

export type GunSolution = z.infer<typeof GunSolutionSchema>;

export const WorkerRequestSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('solve'), parameters: GunParametersSchema }),
]);

export type WorkerRequest = z.infer<typeof WorkerRequestSchema>;

export const WorkerResponseSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('success'), solution: GunSolutionSchema, duration_ms: z.number() }),
  z.object({ type: z.literal('error'), message: z.string() }),
]);

export type WorkerResponse = z.infer<typeof WorkerResponseSchema>;

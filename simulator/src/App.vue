<script setup lang="ts">
import { reactive, computed } from "vue";
import { SidebarLayout, NumberInput } from "@cfasim-ui/components";
import { LineChart, DataTable } from "@cfasim-ui/charts";
import type { Series, ColumnConfig } from "@cfasim-ui/charts";
import { useModel } from "@cfasim-ui/wasm";
import defaults from "../../params/default.toml";

const params = reactive({
  population_size: defaults.population_size as number,
  initial_infections: defaults.initial_infections as number,
  max_time: defaults.max_time as number,
  seed: defaults.seed as number,
  infection_rate: { ...(defaults.infection_rate as { shape: number; rate: number }) },
  infection_duration: { ...(defaults.infection_duration as { shape: number; rate: number }) },
});
const { useOutputs } = useModel("simulator");
const { outputs, loading } = useOutputs("simulate", params);

const incidenceSeries = computed<Series[]>(() => {
  if (!outputs.value?.daily_incidence) return [];
  return [{ data: Array.from(outputs.value.daily_incidence.column("incidence")), color: "#e74c3c" }];
});

const cumulativeSeries = computed<Series[]>(() => {
  if (!outputs.value?.daily_incidence) return [];
  return [{ data: Array.from(outputs.value.daily_incidence.column("cumulative_incidence")), color: "#2980b9" }];
});

const statsColumns: Record<string, ColumnConfig> = {
  total_infections: { label: "Total Infections" },
  attack_rate: { label: "Attack Rate" },
  forecasts_rejected: { label: "Forecasts Rejected" },
  forecast_efficiency: { label: "Forecast Efficiency" },
};
</script>

<template>
  <SidebarLayout>
    <template #sidebar>
      <h2>Basic Transmission</h2>
      <NumberInput v-model="params.population_size" label="Population Size" />
      <NumberInput v-model="params.initial_infections" label="Initial Infections" />
      <NumberInput v-model="params.max_time" label="Max Time" />
      <NumberInput v-model="params.seed" label="Seed" />
      <h2>Infection Rates</h2>
      <p class="note">Each person's rate is drawn from a Gamma distribution with shape k and rate λ.</p>
      <div class="row">
        <NumberInput v-model="params.infection_rate.shape" label="Shape (k)" :step="0.1" />
        <NumberInput v-model="params.infection_rate.rate" label="Rate (λ)" :step="0.05" />
      </div>
      <h2>Infection Durations</h2>
      <p class="note">Each person's duration of infection (i.e., time from infection to recovery) is drawn from a Gamma
        distribution with shape k and rate λ.</p>
      <div class="row">
        <NumberInput v-model="params.infection_duration.shape" label="Shape (k)" :step="0.1" />
        <NumberInput v-model="params.infection_duration.rate" label="Rate (λ)" :step="0.1" />
      </div>
    </template>
    <p v-if="loading">Running simulation...</p>
    <template v-else-if="outputs?.daily_incidence">
      <LineChart :series="incidenceSeries" title="Daily Incidence" xLabel="Day" yLabel="New Infections" :height="300" />
      <LineChart :series="cumulativeSeries" title="Cumulative Incidence" xLabel="Day" yLabel="Total Infections"
        :height="300" />
      <DataTable v-if="outputs?.stats" :data="outputs.stats" :columnConfig="statsColumns" />
    </template>
  </SidebarLayout>
</template>

<style scoped>
.note {
  margin-top: 0;
}

.row {
  display: flex;
  gap: 8px;
}
</style>

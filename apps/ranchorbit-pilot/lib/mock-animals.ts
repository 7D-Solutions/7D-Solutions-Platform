// Mock livestock data for the RanchOrbit pilot.
// Replace with real API calls once the platform backend is wired up.

export type AnimalSex = "male" | "female";
export type AnimalStatus = "healthy" | "monitor" | "treatment" | "quarantine";

export interface Animal {
  id: string;
  tagNumber: string;
  name?: string;
  species: "cattle" | "sheep" | "horse" | "goat";
  breed: string;
  sex: AnimalSex;
  dob: string; // ISO date
  weightKg: number;
  pasture: string;
  status: AnimalStatus;
  lastExamDate: string; // ISO date
  notes?: string;
}

export const MOCK_ANIMALS: Animal[] = [
  {
    id: "a-001",
    tagNumber: "RO-1042",
    name: "Duchess",
    species: "cattle",
    breed: "Angus",
    sex: "female",
    dob: "2020-03-15",
    weightKg: 548,
    pasture: "North Pasture",
    status: "healthy",
    lastExamDate: "2026-03-10",
    notes: "Top producer. Due for booster in June.",
  },
  {
    id: "a-002",
    tagNumber: "RO-1043",
    name: "Buck",
    species: "cattle",
    breed: "Hereford",
    sex: "male",
    dob: "2019-07-22",
    weightKg: 780,
    pasture: "South Pasture",
    status: "healthy",
    lastExamDate: "2026-02-28",
  },
  {
    id: "a-003",
    tagNumber: "RO-2101",
    species: "sheep",
    breed: "Merino",
    sex: "female",
    dob: "2022-01-08",
    weightKg: 68,
    pasture: "East Paddock",
    status: "monitor",
    lastExamDate: "2026-04-01",
    notes: "Mild lameness in right front leg. Re-check in 10 days.",
  },
  {
    id: "a-004",
    tagNumber: "RO-2102",
    species: "sheep",
    breed: "Merino",
    sex: "female",
    dob: "2022-01-08",
    weightKg: 71,
    pasture: "East Paddock",
    status: "healthy",
    lastExamDate: "2026-03-20",
  },
  {
    id: "a-005",
    tagNumber: "RO-3001",
    name: "Ranger",
    species: "horse",
    breed: "Quarter Horse",
    sex: "male",
    dob: "2016-05-10",
    weightKg: 495,
    pasture: "Stable",
    status: "treatment",
    lastExamDate: "2026-04-03",
    notes: "Prescribed antibiotics for respiratory infection. Day 3/7.",
  },
  {
    id: "a-006",
    tagNumber: "RO-1044",
    name: "Rosie",
    species: "cattle",
    breed: "Angus",
    sex: "female",
    dob: "2021-09-30",
    weightKg: 412,
    pasture: "North Pasture",
    status: "healthy",
    lastExamDate: "2026-03-10",
  },
  {
    id: "a-007",
    tagNumber: "RO-4001",
    species: "goat",
    breed: "Boer",
    sex: "male",
    dob: "2023-02-14",
    weightKg: 52,
    pasture: "West Paddock",
    status: "quarantine",
    lastExamDate: "2026-04-05",
    notes: "New arrival — 14-day quarantine until 2026-04-19.",
  },
  {
    id: "a-008",
    tagNumber: "RO-4002",
    species: "goat",
    breed: "Boer",
    sex: "female",
    dob: "2023-02-14",
    weightKg: 44,
    pasture: "West Paddock",
    status: "quarantine",
    lastExamDate: "2026-04-05",
    notes: "New arrival — 14-day quarantine until 2026-04-19.",
  },
];

export function getAnimalById(id: string): Animal | undefined {
  return MOCK_ANIMALS.find((a) => a.id === id);
}

// Summary stats used on the dashboard
export function getAnimalStats() {
  const total = MOCK_ANIMALS.length;
  const byStatus = MOCK_ANIMALS.reduce<Record<AnimalStatus, number>>(
    (acc, a) => {
      acc[a.status] = (acc[a.status] ?? 0) + 1;
      return acc;
    },
    { healthy: 0, monitor: 0, treatment: 0, quarantine: 0 }
  );
  const bySpecies = MOCK_ANIMALS.reduce<Record<string, number>>((acc, a) => {
    acc[a.species] = (acc[a.species] ?? 0) + 1;
    return acc;
  }, {});
  return { total, byStatus, bySpecies };
}

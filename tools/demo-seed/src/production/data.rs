//! Static routing definitions for aerospace manufacturing

// ---------------------------------------------------------------------------
// Routing data types
// ---------------------------------------------------------------------------

pub(super) struct RoutingStepDef {
    /// Work center code (resolved to UUID at runtime)
    pub workcenter_code: &'static str,
    pub operation_name: &'static str,
    pub description: &'static str,
    pub setup_time_minutes: i32,
    pub run_time_minutes: i32,
}

pub(super) struct RoutingDef {
    /// SKU of the make item (resolved to UUID at runtime)
    pub item_sku: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub steps: &'static [RoutingStepDef],
}

// ---------------------------------------------------------------------------
// Static seed data
// ---------------------------------------------------------------------------

pub(super) const ROUTINGS: &[RoutingDef] = &[
    RoutingDef {
        item_sku: "TBB-ASSY-001",
        name: "Turbine Blade Blank Routing",
        description: "5-step HPT blade blank: rough mill → finish mill → heat treat → grind → NDT",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "Rough Mill",
                description: "Rough CNC milling of blade blank profile",
                setup_time_minutes: 30,
                run_time_minutes: 45,
            },
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "Finish Mill",
                description: "Finish CNC milling to final blade profile tolerance",
                setup_time_minutes: 15,
                run_time_minutes: 60,
            },
            RoutingStepDef {
                workcenter_code: "HEAT-TREAT",
                operation_name: "Heat Treat",
                description: "Solution heat treat and age per AMS 4928",
                setup_time_minutes: 10,
                run_time_minutes: 480,
            },
            RoutingStepDef {
                workcenter_code: "GRIND-01",
                operation_name: "Cylindrical Grind",
                description: "Finish grind root and platform surfaces",
                setup_time_minutes: 15,
                run_time_minutes: 30,
            },
            RoutingStepDef {
                workcenter_code: "NDT-01",
                operation_name: "NDT Inspection",
                description: "FPI and UT inspection per ASTM E1444",
                setup_time_minutes: 5,
                run_time_minutes: 20,
            },
        ],
    },
    RoutingDef {
        item_sku: "EMB-ASSY-001",
        name: "Engine Mount Bracket Routing",
        description: "3-step bracket: CNC mill → heat treat → NDT",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "CNC Mill",
                description: "Machine bracket from billet stock",
                setup_time_minutes: 20,
                run_time_minutes: 35,
            },
            RoutingStepDef {
                workcenter_code: "HEAT-TREAT",
                operation_name: "Heat Treat",
                description: "Stress relieve and age harden",
                setup_time_minutes: 10,
                run_time_minutes: 360,
            },
            RoutingStepDef {
                workcenter_code: "NDT-01",
                operation_name: "NDT Inspection",
                description: "Magnetic particle inspection per ASTM E1444",
                setup_time_minutes: 5,
                run_time_minutes: 15,
            },
        ],
    },
    RoutingDef {
        item_sku: "SRA-ASSY-001",
        name: "Structural Rib Assembly Routing",
        description: "4-step rib: CNC mill → lathe → heat treat → assembly",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "CNC Mill",
                description: "Machine rib web and flanges from plate",
                setup_time_minutes: 25,
                run_time_minutes: 50,
            },
            RoutingStepDef {
                workcenter_code: "CNC-LATHE-01",
                operation_name: "CNC Lathe",
                description: "Turn bushings and sleeve inserts",
                setup_time_minutes: 15,
                run_time_minutes: 30,
            },
            RoutingStepDef {
                workcenter_code: "HEAT-TREAT",
                operation_name: "Heat Treat",
                description: "Precipitation hardening cycle",
                setup_time_minutes: 10,
                run_time_minutes: 240,
            },
            RoutingStepDef {
                workcenter_code: "ASSEMBLY-01",
                operation_name: "Assembly",
                description: "Rivet and assemble rib components per drawing",
                setup_time_minutes: 10,
                run_time_minutes: 45,
            },
        ],
    },
    RoutingDef {
        item_sku: "FLC-ASSY-001",
        name: "Fuel Line Connector Routing",
        description: "3-step connector: lathe → grind → NDT",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-LATHE-01",
                operation_name: "CNC Lathe",
                description: "Turn connector body and thread AN fittings",
                setup_time_minutes: 15,
                run_time_minutes: 20,
            },
            RoutingStepDef {
                workcenter_code: "GRIND-01",
                operation_name: "Cylindrical Grind",
                description: "Finish grind sealing surfaces",
                setup_time_minutes: 10,
                run_time_minutes: 15,
            },
            RoutingStepDef {
                workcenter_code: "NDT-01",
                operation_name: "NDT Inspection",
                description: "Dye penetrant inspection of sealing surfaces",
                setup_time_minutes: 5,
                run_time_minutes: 10,
            },
        ],
    },
    RoutingDef {
        item_sku: "LGA-ASSY-001",
        name: "Landing Gear Actuator Housing Routing",
        description: "5-step housing: CNC mill → heat treat → grind → NDT → assembly",
        steps: &[
            RoutingStepDef {
                workcenter_code: "CNC-MILL-01",
                operation_name: "CNC Mill",
                description: "Machine housing bore and mounting faces from forging",
                setup_time_minutes: 30,
                run_time_minutes: 90,
            },
            RoutingStepDef {
                workcenter_code: "HEAT-TREAT",
                operation_name: "Heat Treat",
                description: "Full heat treatment cycle per material spec",
                setup_time_minutes: 10,
                run_time_minutes: 480,
            },
            RoutingStepDef {
                workcenter_code: "GRIND-01",
                operation_name: "Cylindrical Grind",
                description: "Precision grind bore and piston surfaces",
                setup_time_minutes: 15,
                run_time_minutes: 45,
            },
            RoutingStepDef {
                workcenter_code: "NDT-01",
                operation_name: "NDT Inspection",
                description: "Ultrasonic and FPI inspection of critical surfaces",
                setup_time_minutes: 5,
                run_time_minutes: 30,
            },
            RoutingStepDef {
                workcenter_code: "ASSEMBLY-01",
                operation_name: "Final Assembly",
                description: "Install seals, bushings, and hydraulic ports",
                setup_time_minutes: 15,
                run_time_minutes: 60,
            },
        ],
    },
];

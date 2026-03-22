//! Static BOM structure definitions for aerospace manufacturing

// ---------------------------------------------------------------------------
// BOM data types
// ---------------------------------------------------------------------------

pub(super) struct BomComponentDef {
    pub component_sku: &'static str,
    pub quantity: f64,
    pub uom: Option<&'static str>,
    pub scrap_factor: Option<f64>,
    pub find_number: i32,
}

pub(super) struct BomDef {
    pub make_item_sku: &'static str,
    pub description: &'static str,
    pub components: &'static [BomComponentDef],
}

// ---------------------------------------------------------------------------
// Static seed data
// ---------------------------------------------------------------------------

pub(super) const BOMS: &[BomDef] = &[
    BomDef {
        make_item_sku: "TBB-ASSY-001",
        description: "Turbine Blade Blank BOM",
        components: &[BomComponentDef {
            component_sku: "TI64-BAR-001",
            quantity: 1.0,
            uom: Some("KG"),
            scrap_factor: Some(0.05),
            find_number: 10,
        }],
    },
    BomDef {
        make_item_sku: "EMB-ASSY-001",
        description: "Engine Mount Bracket BOM",
        components: &[
            BomComponentDef {
                component_sku: "AL7075-SHT-001",
                quantity: 2.0,
                uom: Some("KG"),
                scrap_factor: None,
                find_number: 10,
            },
            BomComponentDef {
                component_sku: "AN3-BOLT",
                quantity: 4.0,
                uom: Some("EA"),
                scrap_factor: None,
                find_number: 20,
            },
            BomComponentDef {
                component_sku: "MS21042-NUT",
                quantity: 4.0,
                uom: Some("EA"),
                scrap_factor: None,
                find_number: 30,
            },
            BomComponentDef {
                component_sku: "NAS1149-WASH",
                quantity: 8.0,
                uom: Some("EA"),
                scrap_factor: None,
                find_number: 40,
            },
        ],
    },
    BomDef {
        make_item_sku: "SRA-ASSY-001",
        description: "Structural Rib Assembly BOM",
        components: &[
            BomComponentDef {
                component_sku: "4130-TUB-001",
                quantity: 3.0,
                uom: Some("M"),
                scrap_factor: None,
                find_number: 10,
            },
            BomComponentDef {
                component_sku: "INC718-FRG-001",
                quantity: 1.0,
                uom: Some("KG"),
                scrap_factor: Some(0.03),
                find_number: 20,
            },
        ],
    },
    BomDef {
        make_item_sku: "FLC-ASSY-001",
        description: "Fuel Line Connector BOM",
        components: &[
            BomComponentDef {
                component_sku: "4130-TUB-001",
                quantity: 1.0,
                uom: Some("M"),
                scrap_factor: None,
                find_number: 10,
            },
            BomComponentDef {
                component_sku: "AN3-BOLT",
                quantity: 2.0,
                uom: Some("EA"),
                scrap_factor: None,
                find_number: 20,
            },
        ],
    },
    BomDef {
        make_item_sku: "LGA-ASSY-001",
        description: "Landing Gear Actuator Housing BOM",
        components: &[
            BomComponentDef {
                component_sku: "INC718-FRG-001",
                quantity: 2.0,
                uom: Some("KG"),
                scrap_factor: None,
                find_number: 10,
            },
            BomComponentDef {
                component_sku: "TI64-BAR-001",
                quantity: 1.0,
                uom: Some("KG"),
                scrap_factor: None,
                find_number: 20,
            },
        ],
    },
];

/// All SKUs referenced in BOM definitions (make items + components)
pub(super) const ALL_REFERENCED_SKUS: &[&str] = &[
    "TBB-ASSY-001",
    "EMB-ASSY-001",
    "SRA-ASSY-001",
    "FLC-ASSY-001",
    "LGA-ASSY-001",
    "TI64-BAR-001",
    "INC718-FRG-001",
    "AL7075-SHT-001",
    "4130-TUB-001",
    "AN3-BOLT",
    "MS21042-NUT",
    "NAS1149-WASH",
];

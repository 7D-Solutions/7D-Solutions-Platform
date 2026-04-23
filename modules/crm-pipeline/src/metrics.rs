use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts, Registry,
    TextEncoder,
};

pub struct CrmPipelineMetrics {
    pub leads_created_total: IntCounter,
    pub leads_converted_total: IntCounter,
    pub opportunities_created_total: IntCounter,
    pub opportunities_closed_won_total: IntCounter,
    pub opportunities_closed_lost_total: IntCounter,
    pub activities_logged_total: IntCounter,
    pub open_opportunities_count: IntGauge,
    pub open_leads_count: IntGauge,
    pub http_request_duration_seconds: HistogramVec,
    pub http_requests_total: IntCounterVec,
    registry: Registry,
}

impl CrmPipelineMetrics {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        let leads_created_total =
            IntCounter::new("crm_leads_created_total", "Total CRM leads created")?;
        registry.register(Box::new(leads_created_total.clone()))?;

        let leads_converted_total =
            IntCounter::new("crm_leads_converted_total", "Total CRM leads converted")?;
        registry.register(Box::new(leads_converted_total.clone()))?;

        let opportunities_created_total = IntCounter::new(
            "crm_opportunities_created_total",
            "Total opportunities created",
        )?;
        registry.register(Box::new(opportunities_created_total.clone()))?;

        let opportunities_closed_won_total = IntCounter::new(
            "crm_opportunities_closed_won_total",
            "Total opportunities closed won",
        )?;
        registry.register(Box::new(opportunities_closed_won_total.clone()))?;

        let opportunities_closed_lost_total = IntCounter::new(
            "crm_opportunities_closed_lost_total",
            "Total opportunities closed lost",
        )?;
        registry.register(Box::new(opportunities_closed_lost_total.clone()))?;

        let activities_logged_total =
            IntCounter::new("crm_activities_logged_total", "Total activities logged")?;
        registry.register(Box::new(activities_logged_total.clone()))?;

        let open_opportunities_count =
            IntGauge::new("crm_open_opportunities_count", "Current open opportunities")?;
        registry.register(Box::new(open_opportunities_count.clone()))?;

        let open_leads_count = IntGauge::new(
            "crm_open_leads_count",
            "Current active (non-terminal) leads",
        )?;
        registry.register(Box::new(open_leads_count.clone()))?;

        let http_request_duration_seconds = HistogramVec::new(
            HistogramOpts::new("crm_http_request_duration_seconds", "HTTP request latency")
                .buckets(vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5]),
            &["method", "path", "status"],
        )?;
        registry.register(Box::new(http_request_duration_seconds.clone()))?;

        let http_requests_total = IntCounterVec::new(
            Opts::new("crm_http_requests_total", "Total HTTP requests"),
            &["method", "path", "status"],
        )?;
        registry.register(Box::new(http_requests_total.clone()))?;

        Ok(Self {
            leads_created_total,
            leads_converted_total,
            opportunities_created_total,
            opportunities_closed_won_total,
            opportunities_closed_lost_total,
            activities_logged_total,
            open_opportunities_count,
            open_leads_count,
            http_request_duration_seconds,
            http_requests_total,
            registry,
        })
    }

    pub fn render(&self) -> String {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buf = Vec::new();
        encoder
            .encode(&metric_families, &mut buf)
            .unwrap_or_default();
        String::from_utf8(buf).unwrap_or_default()
    }
}

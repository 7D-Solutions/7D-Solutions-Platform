use prometheus::{Counter, Histogram, HistogramOpts, Opts, Registry};

pub struct CcMetrics {
    pub complaints_created: Counter,
    pub complaints_triaged: Counter,
    pub complaints_investigated: Counter,
    pub complaints_responded: Counter,
    pub complaints_closed: Counter,
    pub complaints_cancelled: Counter,
    pub activity_entries_recorded: Counter,
    pub resolutions_recorded: Counter,
    pub request_duration_seconds: Histogram,
}

impl CcMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let complaints_created = Counter::with_opts(Opts::new(
            "cc_complaints_created_total",
            "Total complaints created",
        ))?;
        registry.register(Box::new(complaints_created.clone()))?;

        let complaints_triaged = Counter::with_opts(Opts::new(
            "cc_complaints_triaged_total",
            "Total complaints triaged",
        ))?;
        registry.register(Box::new(complaints_triaged.clone()))?;

        let complaints_investigated = Counter::with_opts(Opts::new(
            "cc_complaints_investigated_total",
            "Total complaints moved to investigating",
        ))?;
        registry.register(Box::new(complaints_investigated.clone()))?;

        let complaints_responded = Counter::with_opts(Opts::new(
            "cc_complaints_responded_total",
            "Total complaints marked responded",
        ))?;
        registry.register(Box::new(complaints_responded.clone()))?;

        let complaints_closed = Counter::with_opts(Opts::new(
            "cc_complaints_closed_total",
            "Total complaints closed",
        ))?;
        registry.register(Box::new(complaints_closed.clone()))?;

        let complaints_cancelled = Counter::with_opts(Opts::new(
            "cc_complaints_cancelled_total",
            "Total complaints cancelled",
        ))?;
        registry.register(Box::new(complaints_cancelled.clone()))?;

        let activity_entries_recorded = Counter::with_opts(Opts::new(
            "cc_activity_entries_total",
            "Total activity log entries recorded",
        ))?;
        registry.register(Box::new(activity_entries_recorded.clone()))?;

        let resolutions_recorded = Counter::with_opts(Opts::new(
            "cc_resolutions_recorded_total",
            "Total complaint resolutions recorded",
        ))?;
        registry.register(Box::new(resolutions_recorded.clone()))?;

        let request_duration_seconds = Histogram::with_opts(HistogramOpts::new(
            "cc_request_duration_seconds",
            "HTTP request duration in seconds",
        ))?;
        registry.register(Box::new(request_duration_seconds.clone()))?;

        Ok(Self {
            complaints_created,
            complaints_triaged,
            complaints_investigated,
            complaints_responded,
            complaints_closed,
            complaints_cancelled,
            activity_entries_recorded,
            resolutions_recorded,
            request_duration_seconds,
        })
    }
}

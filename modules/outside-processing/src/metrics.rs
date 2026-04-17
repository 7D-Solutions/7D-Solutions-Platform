use prometheus::{Counter, Histogram, HistogramOpts, Opts, Registry};

pub struct OpMetrics {
    pub orders_created: Counter,
    pub orders_issued: Counter,
    pub orders_closed: Counter,
    pub orders_cancelled: Counter,
    pub ship_events_recorded: Counter,
    pub return_events_recorded: Counter,
    pub reviews_recorded: Counter,
    pub request_duration_seconds: Histogram,
}

impl OpMetrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let orders_created = Counter::with_opts(Opts::new(
            "op_orders_created_total",
            "Total OP orders created",
        ))?;
        registry.register(Box::new(orders_created.clone()))?;

        let orders_issued = Counter::with_opts(Opts::new(
            "op_orders_issued_total",
            "Total OP orders issued",
        ))?;
        registry.register(Box::new(orders_issued.clone()))?;

        let orders_closed = Counter::with_opts(Opts::new(
            "op_orders_closed_total",
            "Total OP orders closed",
        ))?;
        registry.register(Box::new(orders_closed.clone()))?;

        let orders_cancelled = Counter::with_opts(Opts::new(
            "op_orders_cancelled_total",
            "Total OP orders cancelled",
        ))?;
        registry.register(Box::new(orders_cancelled.clone()))?;

        let ship_events_recorded = Counter::with_opts(Opts::new(
            "op_ship_events_total",
            "Total ship events recorded",
        ))?;
        registry.register(Box::new(ship_events_recorded.clone()))?;

        let return_events_recorded = Counter::with_opts(Opts::new(
            "op_return_events_total",
            "Total return events recorded",
        ))?;
        registry.register(Box::new(return_events_recorded.clone()))?;

        let reviews_recorded = Counter::with_opts(Opts::new(
            "op_reviews_recorded_total",
            "Total vendor reviews recorded",
        ))?;
        registry.register(Box::new(reviews_recorded.clone()))?;

        let request_duration_seconds = Histogram::with_opts(HistogramOpts::new(
            "op_request_duration_seconds",
            "HTTP request duration in seconds",
        ))?;
        registry.register(Box::new(request_duration_seconds.clone()))?;

        Ok(Self {
            orders_created,
            orders_issued,
            orders_closed,
            orders_cancelled,
            ship_events_recorded,
            return_events_recorded,
            reviews_recorded,
            request_duration_seconds,
        })
    }
}

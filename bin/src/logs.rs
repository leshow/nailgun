use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    fmt::{self, format::Pretty},
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

use crate::args::{Args, LogStructure};

// TODO: custom logging output format?
// struct Output;
// impl<'writer> FormatFields<'writer> for Output {
//     fn format_fields<R: tracing_subscriber::prelude::__tracing_subscriber_field_RecordFields>(
//         &self,
//         writer: &'writer mut dyn std::fmt::Write,
//         fields: R,
//     ) -> std::fmt::Result {
//         todo!()
//     }
// }

pub fn setup(args: &Args) -> Result<Option<WorkerGuard>> {
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))?
        .add_directive("tokio_util=off".parse()?)
        .add_directive("h2=off".parse()?);
    let registry = tracing_subscriber::registry().with(filter_layer);

    // this code duplication is unfortunate-- but it looks
    // like tracing doesn't support dynamically configured subscribers
    Ok(match &args.log_file {
        Some(file) => {
            let appender = tracing_appender::rolling::never(
                file.parent()
                    .with_context(|| format!("failed to start log on file {:#?}", file))?,
                file.file_name()
                    .with_context(|| format!("failed to start log on file {:#?}", file))?,
            );

            let (non_blocking_appender, _guard) = tracing_appender::non_blocking(appender);

            match args.output {
                LogStructure::Pretty => {
                    registry
                        .with(
                            fmt::layer()
                                .fmt_fields(Pretty::with_source_location(Pretty::default(), false))
                                .with_target(false),
                        )
                        .with(
                            fmt::layer()
                                .fmt_fields(Pretty::with_source_location(Pretty::default(), false))
                                .with_target(false)
                                .with_writer(non_blocking_appender),
                        )
                        .init();
                }
                LogStructure::Debug => {
                    registry
                        .with(fmt::layer())
                        .with(fmt::layer().with_writer(non_blocking_appender))
                        .init();
                }
                LogStructure::Json => {
                    registry
                        .with(fmt::layer().json())
                        .with(fmt::layer().json().with_writer(non_blocking_appender))
                        .init();
                }
            }
            Some(_guard)
        }
        None => {
            match args.output {
                LogStructure::Pretty => {
                    registry
                        .with(
                            fmt::layer()
                                .fmt_fields(Pretty::with_source_location(Pretty::default(), false))
                                .with_target(false),
                        )
                        .init();
                }
                LogStructure::Debug => {
                    registry.with(fmt::layer()).init();
                }
                LogStructure::Json => {
                    registry.with(fmt::layer().json()).init();
                }
            }
            None
        }
    })
}

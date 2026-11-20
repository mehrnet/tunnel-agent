use super::{MAX_LANES, MAX_RATE, TelemetryReport};

pub(super) fn valid_report(report: &TelemetryReport) -> bool {
    report.sampled_at > 0
        && report.host.cpu_milli_percent <= 100_000
        && report.host.memory_milli_percent <= 100_000
        && report.host.rx_bytes_per_sec <= MAX_RATE
        && report.host.tx_bytes_per_sec <= MAX_RATE
        && report.transport_keys.as_ref().is_none_or(|keys| {
            keys.hpnssh
                .as_deref()
                .is_none_or(crate::transport_identity::valid_ssh_key)
                && keys
                    .rathole
                    .as_deref()
                    .is_none_or(crate::transport_identity::valid_noise_key)
        })
        && report.lanes.len() <= MAX_LANES
        && report.lanes.iter().all(|lane| {
            !lane.id.is_empty()
                && lane.id.len() <= 128
                && lane.id.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

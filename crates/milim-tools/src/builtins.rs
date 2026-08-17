//! Built-in tools available out of the box.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use milim_core::{Error, Result};

use crate::{Tool, ToolConcurrency, ToolEffect, ToolUiDescriptor};

/// Max characters returned by `http_fetch`.
const MAX_FETCH_CHARS: usize = 100_000;
const MAX_FETCH_BYTES: usize = 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const MAX_CHART_SERIES: usize = 8;
const MAX_CHART_POINTS: usize = 400;

/// Echoes its arguments back — deterministic, handy for testing tool loops.
pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echo the provided arguments back to the caller."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "text": { "type": "string", "description": "Text to echo." } },
            "additionalProperties": true
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        Ok(json!({ "echoed": args }))
    }
}

/// Returns the current time as unix seconds + RFC-3339-ish UTC string.
pub struct CurrentTimeTool;

#[async_trait]
impl Tool for CurrentTimeTool {
    fn name(&self) -> &str {
        "current_time"
    }

    fn description(&self) -> &str {
        "Get the current UTC time as a unix timestamp."
    }

    fn input_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }

    async fn invoke(&self, _args: Value) -> Result<Value> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(json!({ "unix": now }))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChartSpec {
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subtitle: Option<String>,
    #[serde(rename = "type")]
    chart_type: ChartType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    orientation: Option<ChartOrientation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x_format: Option<ChartNumberFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y_format: Option<ChartNumberFormat>,
    series: Vec<ChartSeries>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ChartType {
    Bar,
    Line,
    Pie,
    Scatter,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ChartOrientation {
    Vertical,
    Horizontal,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChartNumberFormat {
    style: ChartNumberStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    precision: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notation: Option<ChartNumberNotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sign_display: Option<ChartSignDisplay>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ChartNumberStyle {
    Number,
    Percent,
    Currency,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ChartNumberNotation {
    Standard,
    Compact,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum ChartSignDisplay {
    Auto,
    Always,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChartSeries {
    name: String,
    points: Vec<ChartPoint>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChartPoint {
    x: ChartX,
    y: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum ChartX {
    Text(String),
    Number(f64),
}

/// Render a bounded native chart at the exact tool-call position in chat.
pub struct RenderChartTool;

impl RenderChartTool {
    fn validate(spec: &ChartSpec) -> Result<()> {
        validate_chart_text(&spec.title, "title", 160, false)?;
        if let Some(value) = &spec.subtitle {
            validate_chart_text(value, "subtitle", 300, true)?;
        }
        if let Some(value) = &spec.x_label {
            validate_chart_text(value, "x_label", 80, true)?;
        }
        if let Some(value) = &spec.y_label {
            validate_chart_text(value, "y_label", 80, true)?;
        }
        if spec.orientation.is_some() && !matches!(spec.chart_type, ChartType::Bar) {
            return Err(Error::InvalidRequest(
                "orientation is supported only for bar charts".to_string(),
            ));
        }
        if spec.x_format.is_some() && !matches!(spec.chart_type, ChartType::Scatter) {
            return Err(Error::InvalidRequest(
                "x_format is supported only for scatter charts".to_string(),
            ));
        }
        if let Some(format) = &spec.x_format {
            validate_chart_number_format(format, "x_format")?;
        }
        if let Some(format) = &spec.y_format {
            validate_chart_number_format(format, "y_format")?;
        }
        if spec.series.is_empty() || spec.series.len() > MAX_CHART_SERIES {
            return Err(Error::InvalidRequest(format!(
                "series must contain 1 to {MAX_CHART_SERIES} entries"
            )));
        }
        if matches!(spec.chart_type, ChartType::Pie) && spec.series.len() != 1 {
            return Err(Error::InvalidRequest(
                "pie charts require exactly one series".to_string(),
            ));
        }
        if matches!(spec.chart_type, ChartType::Pie) && spec.series[0].points.len() > 12 {
            return Err(Error::InvalidRequest(
                "pie charts support at most 12 slices".to_string(),
            ));
        }
        let mut point_count = 0;
        for series in &spec.series {
            validate_chart_text(&series.name, "series name", 80, false)?;
            if series.points.is_empty() {
                return Err(Error::InvalidRequest(
                    "each series must contain at least one point".to_string(),
                ));
            }
            point_count += series.points.len();
            for point in &series.points {
                if !point.y.is_finite() {
                    return Err(Error::InvalidRequest(
                        "chart values must be finite numbers".to_string(),
                    ));
                }
                match &point.x {
                    ChartX::Text(value) => {
                        validate_chart_text(value, "point x", 80, false)?;
                        if matches!(spec.chart_type, ChartType::Scatter) {
                            return Err(Error::InvalidRequest(
                                "scatter chart x values must be numbers".to_string(),
                            ));
                        }
                    }
                    ChartX::Number(value) if !value.is_finite() => {
                        return Err(Error::InvalidRequest(
                            "chart x values must be finite".to_string(),
                        ));
                    }
                    ChartX::Number(_) => {}
                }
            }
        }
        if point_count > MAX_CHART_POINTS {
            return Err(Error::InvalidRequest(format!(
                "chart data exceeds the {MAX_CHART_POINTS}-point limit"
            )));
        }
        if matches!(spec.chart_type, ChartType::Pie)
            && (spec.series[0].points.iter().any(|point| point.y < 0.0)
                || spec.series[0]
                    .points
                    .iter()
                    .map(|point| point.y)
                    .sum::<f64>()
                    <= 0.0)
        {
            return Err(Error::InvalidRequest(
                "pie chart values must be non-negative with a positive total".to_string(),
            ));
        }
        Ok(())
    }
}

fn validate_chart_text(value: &str, field: &str, max: usize, allow_empty: bool) -> Result<()> {
    let length = value.chars().count();
    if (!allow_empty && value.trim().is_empty()) || length > max {
        return Err(Error::InvalidRequest(format!(
            "{field} must be {}at most {max} characters",
            if allow_empty { "" } else { "non-empty and " }
        )));
    }
    Ok(())
}

fn validate_chart_number_format(format: &ChartNumberFormat, field: &str) -> Result<()> {
    if format.precision.is_some_and(|precision| precision > 4) {
        return Err(Error::InvalidRequest(format!(
            "{field}.precision must be between 0 and 4"
        )));
    }
    match (&format.style, &format.currency) {
        (ChartNumberStyle::Currency, Some(currency))
            if currency.len() == 3
                && currency
                    .bytes()
                    .all(|character| character.is_ascii_uppercase()) => {}
        (ChartNumberStyle::Currency, _) => {
            return Err(Error::InvalidRequest(format!(
                "{field}.currency must be a three-letter uppercase code for currency values"
            )));
        }
        (_, Some(_)) => {
            return Err(Error::InvalidRequest(format!(
                "{field}.currency is allowed only when style is currency"
            )));
        }
        _ => {}
    }
    Ok(())
}

#[async_trait]
impl Tool for RenderChartTool {
    fn name(&self) -> &str {
        "render_chart"
    }

    fn description(&self) -> &str {
        "Render a native inline bar, line, pie, or scatter chart in the Milim transcript. Bar charts may be vertical or horizontal. Use this when the user asks to plot or visualize structured numeric data."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "maxLength": 160 },
                "subtitle": { "type": "string", "maxLength": 300 },
                "type": { "type": "string", "enum": ["bar", "line", "pie", "scatter"] },
                "orientation": { "type": "string", "enum": ["vertical", "horizontal"], "description": "Optional bar direction. Defaults to vertical and is supported only for bar charts." },
                "x_label": { "type": "string", "maxLength": 80 },
                "y_label": { "type": "string", "maxLength": 80 },
                "x_format": chart_number_format_schema("Format numeric scatter x values."),
                "y_format": chart_number_format_schema("Format y values. Percent values use percentage points, so 2.5 renders as 2.5%."),
                "series": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_CHART_SERIES,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "maxLength": 80 },
                            "points": {
                                "type": "array",
                                "minItems": 1,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "x": { "type": ["string", "number"], "description": "Category/time label, or a numeric x value for scatter charts." },
                                        "y": { "type": "number" }
                                    },
                                    "required": ["x", "y"],
                                    "additionalProperties": false
                                }
                            }
                        },
                        "required": ["name", "points"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["title", "type", "series"],
            "additionalProperties": false
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn ui(&self) -> Option<ToolUiDescriptor> {
        Some(ToolUiDescriptor::NativeChart)
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let spec: ChartSpec = serde_json::from_value(args)
            .map_err(|error| Error::InvalidRequest(format!("invalid chart: {error}")))?;
        Self::validate(&spec)?;
        serde_json::to_value(spec).map_err(Into::into)
    }
}

fn chart_number_format_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "properties": {
            "style": { "type": "string", "enum": ["number", "percent", "currency"] },
            "currency": { "type": "string", "pattern": "^[A-Z]{3}$" },
            "precision": { "type": "integer", "minimum": 0, "maximum": 4 },
            "notation": { "type": "string", "enum": ["standard", "compact"] },
            "sign_display": { "type": "string", "enum": ["auto", "always"] }
        },
        "required": ["style"],
        "additionalProperties": false
    })
}

/// Fetch an `http(s)` URL and return its status + (truncated) body.
pub struct HttpFetchTool;

#[async_trait]
impl Tool for HttpFetchTool {
    fn name(&self) -> &str {
        "http_fetch"
    }

    fn description(&self) -> &str {
        "Fetch an http(s) URL and return its status code and text body."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "url": { "type": "string", "description": "The http(s) URL to fetch." } },
            "required": ["url"]
        })
    }

    fn effect(&self) -> ToolEffect {
        ToolEffect::ReadOnly
    }

    fn concurrency(&self) -> ToolConcurrency {
        ToolConcurrency::Parallel
    }

    async fn invoke(&self, args: Value) -> Result<Value> {
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::InvalidRequest("missing 'url' argument".to_string()))?;
        let mut url = reqwest::Url::parse(url)
            .map_err(|error| Error::InvalidRequest(format!("invalid URL: {error}")))?;
        let mut resp = None;
        for redirect in 0..=MAX_REDIRECTS {
            let client = public_http_client(&url).await?;
            let response = client
                .get(url.clone())
                .send()
                .await
                .map_err(|e| Error::Upstream(e.to_string()))?;
            if response.status().is_redirection() {
                if redirect == MAX_REDIRECTS {
                    return Err(Error::Upstream("too many redirects".to_string()));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or_else(|| Error::Upstream("redirect is missing Location".to_string()))?;
                url = url
                    .join(location)
                    .map_err(|error| Error::Upstream(format!("invalid redirect: {error}")))?;
                continue;
            }
            resp = Some(response);
            break;
        }
        let mut resp = resp.ok_or_else(|| Error::Upstream("request failed".to_string()))?;
        let status = resp.status().as_u16();
        let mut bytes = Vec::new();
        let mut body_truncated = false;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|error| Error::Upstream(error.to_string()))?
        {
            let remaining = MAX_FETCH_BYTES.saturating_sub(bytes.len());
            if chunk.len() > remaining {
                bytes.extend_from_slice(&chunk[..remaining]);
                body_truncated = true;
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&bytes);
        let truncated: String = body.chars().take(MAX_FETCH_CHARS).collect();
        body_truncated |= body.chars().count() > MAX_FETCH_CHARS;
        Ok(json!({ "status": status, "body": truncated, "truncated": body_truncated }))
    }
}

async fn public_http_client(url: &reqwest::Url) -> Result<reqwest::Client> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::InvalidRequest(
            "only http(s) URLs are allowed".to_string(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| Error::InvalidRequest("URL must include a host".to_string()))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::InvalidRequest("URL must include a valid port".to_string()))?;
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| Error::Upstream(format!("DNS lookup failed: {error}")))?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(Error::InvalidRequest(
            "private, local, and link-local network addresses are not allowed".to_string(),
        ));
    }
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none());
    for address in addresses {
        builder = builder.resolve(host, address);
    }
    builder
        .build()
        .map_err(|error| Error::Other(format!("HTTP client: {error}")))
}

fn is_public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || ip.octets()[0] >= 240
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])))
        }
        std::net::IpAddr::V6(ip) => {
            let octets = ip.octets();
            if octets[..10] == [0; 10] && octets[10..12] == [0xff, 0xff] {
                return is_public_ip(std::net::IpAddr::V4(std::net::Ipv4Addr::new(
                    octets[12], octets[13], octets[14], octets[15],
                )));
            }
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (octets[0] & 0xfe) == 0xfc
                || (octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_address_policy_rejects_local_networks() {
        assert!(!is_public_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("10.0.0.1".parse().unwrap()));
        assert!(!is_public_ip("169.254.169.254".parse().unwrap()));
        assert!(!is_public_ip("fc00::1".parse().unwrap()));
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn render_chart_validates_and_normalizes_specs() {
        let tool = RenderChartTool;
        let legacy = tool
            .invoke(json!({
                "title": "Weekly usage",
                "type": "line",
                "series": [{
                    "name": "Requests",
                    "points": [{ "x": "Mon", "y": 12 }, { "x": "Tue", "y": 18 }]
                }]
            }))
            .await
            .unwrap();
        assert_eq!(legacy["type"], "line");
        assert!(legacy.get("orientation").is_none());
        assert!(legacy.get("y_format").is_none());
        assert!(matches!(tool.ui(), Some(ToolUiDescriptor::NativeChart)));

        let horizontal = tool
            .invoke(json!({
                "title": "Ranked values",
                "type": "bar",
                "orientation": "horizontal",
                "series": [{ "name": "Values", "points": [{ "x": "Long category", "y": 12 }] }]
            }))
            .await
            .unwrap();
        assert_eq!(horizontal["orientation"], "horizontal");

        for format in [
            json!({ "style": "number", "precision": 1, "notation": "compact", "sign_display": "always" }),
            json!({ "style": "percent", "precision": 2 }),
            json!({ "style": "currency", "currency": "USD" }),
        ] {
            let valid = tool
                .invoke(json!({
                    "title": "Formatted values",
                    "type": "scatter",
                    "x_format": { "style": "number", "notation": "compact" },
                    "y_format": format,
                    "series": [{ "name": "Values", "points": [{ "x": 1, "y": 2.5 }] }]
                }))
                .await
                .unwrap();
            assert!(valid.get("x_format").is_some());
            assert!(valid.get("y_format").is_some());
        }

        for invalid in [
            json!({ "style": "currency" }),
            json!({ "style": "currency", "currency": "usd" }),
            json!({ "style": "number", "currency": "USD" }),
            json!({ "style": "number", "precision": 5 }),
            json!({ "style": "number", "extra": true }),
        ] {
            assert!(tool
                .invoke(json!({
                    "title": "Invalid format",
                    "type": "bar",
                    "y_format": invalid,
                    "series": [{ "name": "Values", "points": [{ "x": "A", "y": 1 }] }]
                }))
                .await
                .is_err());
        }
        assert!(tool
            .invoke(json!({
                "title": "Invalid orientation",
                "type": "line",
                "orientation": "horizontal",
                "series": [{ "name": "Values", "points": [{ "x": "A", "y": 1 }] }]
            }))
            .await
            .is_err());
        assert!(tool
            .invoke(json!({
                "title": "Unknown orientation",
                "type": "bar",
                "orientation": "diagonal",
                "series": [{ "name": "Values", "points": [{ "x": "A", "y": 1 }] }]
            }))
            .await
            .is_err());
        assert!(tool
            .invoke(json!({
                "title": "Invalid x format",
                "type": "line",
                "x_format": { "style": "number" },
                "series": [{ "name": "Values", "points": [{ "x": "A", "y": 1 }] }]
            }))
            .await
            .is_err());
        assert!(tool
            .invoke(json!({
                "title": "Invalid scatter",
                "type": "scatter",
                "series": [{ "name": "Points", "points": [{ "x": "A", "y": 1 }] }]
            }))
            .await
            .is_err());
    }
}

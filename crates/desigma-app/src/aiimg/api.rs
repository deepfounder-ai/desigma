//! The OpenRouter Images API: list the image models, generate one image.
//!
//! `POST {base}/images` takes a model, a prompt and optional reference images (as data URLs) and
//! answers with base64 images; `GET {base}/images/models` lists the models that can do that.
//! Everything here is blocking and runs on the job thread.

use base64::Engine;
use serde_json::{json, Value};
use std::io::Read;
use std::time::Duration;

pub const BASE: &str = "https://openrouter.ai/api/v1";
/// Generation can take a minute or two.
const TIMEOUT: Duration = Duration::from_secs(240);
const MAX_LISTING: u64 = 8 << 20;
/// Most bytes of one answer (a 4K PNG as base64 stays well below).
const MAX_ANSWER: u64 = 96 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub name: String,
    /// Takes reference images.
    pub references: bool,
}

/// One generation request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub model: String,
    pub prompt: String,
    /// `None` leaves the choice to the provider.
    pub aspect_ratio: Option<String>,
    /// PNG or JPEG files the model should look at.
    pub references: Vec<Vec<u8>>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout(TIMEOUT).build()
}

fn headers(r: ureq::Request, key: &str) -> ureq::Request {
    r.set("Authorization", &format!("Bearer {key}")).set("HTTP-Referer", "https://github.com/deepfounder-ai/desigma").set("X-Title", "desigma")
}

fn read_json(r: ureq::Response, cap: u64) -> Result<Value, String> {
    let mut text = String::new();
    r.into_reader().take(cap + 1).read_to_string(&mut text).map_err(|e| format!("OpenRouter's answer could not be read ({e})"))?;
    if text.len() as u64 > cap {
        return Err("OpenRouter's answer is too large".into());
    }
    serde_json::from_str(&text).map_err(|_| "OpenRouter's answer is not JSON".to_string())
}

/// What went wrong, in words a person can act on.
fn failure(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => {
            let detail = read_json(r, 1 << 20).ok().and_then(|v| v["error"]["message"].as_str().map(str::to_string)).filter(|m| !m.is_empty());
            let lead = match code {
                401 | 403 => "OpenRouter did not accept the API key".to_string(),
                402 => "The OpenRouter account is out of credits".to_string(),
                429 => "OpenRouter is rate limiting this key; try again in a moment".to_string(),
                502 | 503 => "The model did not finish the image; nothing was charged".to_string(),
                _ => format!("OpenRouter answered with error {code}"),
            };
            match detail {
                Some(d) => format!("{lead}: {}", d.chars().take(300).collect::<String>()),
                None => lead,
            }
        }
        ureq::Error::Transport(t) => format!("Could not reach OpenRouter ({t})"),
    }
}

fn modalities(m: &Value, key: &str) -> Vec<String> {
    let list = m["architecture"][key].as_array().or_else(|| m[key].as_array());
    list.map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// The models of a listing answer, by name.
pub fn parse_models(v: &Value) -> Vec<Model> {
    let mut out: Vec<Model> = v["data"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    let id = m["id"].as_str()?.trim().to_string();
                    if id.is_empty() || id.len() > 200 {
                        return None;
                    }
                    let outputs = modalities(m, "output_modalities");
                    if !outputs.is_empty() && !outputs.iter().any(|o| o == "image") {
                        return None;
                    }
                    let name = m["name"].as_str().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&id).chars().take(120).collect();
                    Some(Model { name, references: modalities(m, "input_modalities").iter().any(|i| i == "image"), id })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|m| m.name.to_lowercase());
    out
}

pub fn list_models(base: &str, key: &str) -> Result<Vec<Model>, String> {
    let r = headers(agent().get(&format!("{base}/images/models")), key).call().map_err(failure)?;
    let models = parse_models(&read_json(r, MAX_LISTING)?);
    if models.is_empty() {
        return Err("OpenRouter listed no image models".into());
    }
    Ok(models)
}

fn data_url(bytes: &[u8]) -> String {
    let mime = match image::guess_format(bytes) {
        Ok(image::ImageFormat::Jpeg) => "image/jpeg",
        Ok(image::ImageFormat::WebP) => "image/webp",
        _ => "image/png",
    };
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// The JSON body of a generation request.
pub fn body(req: &Request) -> Value {
    let mut b = json!({"model": req.model, "prompt": req.prompt});
    if let Some(a) = &req.aspect_ratio {
        b["aspect_ratio"] = json!(a);
    }
    if !req.references.is_empty() {
        b["input_references"] = req.references.iter().map(|r| json!({"type":"image_url","image_url":{"url":data_url(r)}})).collect();
    }
    b
}

/// The first image of a generation answer, as file bytes.
pub fn parse_image(v: &Value) -> Result<Vec<u8>, String> {
    let first = v["data"].as_array().and_then(|a| a.first()).ok_or("The model returned no image")?;
    let inline = |u: &str| u.strip_prefix("data:").and_then(|u| u.split_once(";base64,")).map(|(_, d)| d.to_string());
    let b64 = first["b64_json"].as_str().map(str::to_string).or_else(|| first["url"].as_str().and_then(inline)).ok_or("The model returned no image")?;
    base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| "The model returned an image that could not be decoded".to_string())
}

/// Generates one image and returns its file bytes.
pub fn generate(base: &str, key: &str, req: &Request) -> Result<Vec<u8>, String> {
    let r =
        headers(agent().post(&format!("{base}/images")), key).set("Content-Type", "application/json").send_string(&body(req).to_string()).map_err(failure)?;
    parse_image(&read_json(r, MAX_ANSWER)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listing_keeps_image_models_and_marks_reference_support() {
        let v = json!({"data":[
            {"id":"b/second","name":"Zeta","architecture":{"input_modalities":["text","image"],"output_modalities":["image"]}},
            {"id":"a/first","name":"alpha","architecture":{"input_modalities":["text"],"output_modalities":["image"]}},
            {"id":"t/text","name":"Text only","architecture":{"input_modalities":["text"],"output_modalities":["text"]}},
            {"id":"flat/one","input_modalities":["image"],"output_modalities":["image"]},
            {"id":"","name":"no id"},
        ]});
        let m = parse_models(&v);
        assert_eq!(m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a/first", "flat/one", "b/second"]);
        assert_eq!(m.iter().map(|m| m.references).collect::<Vec<_>>(), [false, true, true]);
        assert_eq!(m[1].name, "flat/one", "a model without a name shows its id");
    }

    #[test]
    fn request_body_carries_references_as_data_urls() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let b = body(&Request { model: "m".into(), prompt: "p".into(), aspect_ratio: Some("16:9".into()), references: vec![png.to_vec()] });
        assert_eq!(b["model"], "m");
        assert_eq!(b["aspect_ratio"], "16:9");
        assert_eq!(b["input_references"][0]["type"], "image_url");
        assert_eq!(b["input_references"][0]["image_url"]["url"], "data:image/png;base64,iVBORw0KGgo=");
        let plain = body(&Request { model: "m".into(), prompt: "p".into(), ..Default::default() });
        assert!(plain.get("aspect_ratio").is_none() && plain.get("input_references").is_none());
    }

    #[test]
    fn answers_decode_from_b64_json_or_a_data_url() {
        assert_eq!(parse_image(&json!({"data":[{"b64_json":"aGk=","media_type":"image/png"}]})).unwrap(), b"hi");
        assert_eq!(parse_image(&json!({"data":[{"url":"data:image/png;base64,aGk="}]})).unwrap(), b"hi");
        assert!(parse_image(&json!({"data":[]})).is_err());
        assert!(parse_image(&json!({"data":[{"b64_json":"***"}]})).is_err());
        assert!(parse_image(&json!({"data":[{"url":"https://example.com/x.png"}]})).is_err(), "remote URLs are not fetched");
    }
}

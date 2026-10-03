// Appended to an otherwise unchanged original source module, replacing only its seconds clock.
static MIGRATION_CLOCK: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(1800000000);
fn current_unix_time() -> i64 {
    MIGRATION_CLOCK.load(std::sync::atomic::Ordering::Relaxed)
}
fn migration_identity(value: &Value) -> ZjlibCnkiArticleIdentity {
    ZjlibCnkiArticleIdentity {
        title: value["title"].as_str().unwrap_or_default().to_string(),
        authors: value["authors"].as_str().unwrap_or_default().to_string(),
        journal_title: value["journal_title"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    }
}
fn migration_outcome(result: Result<Value, ZjlibCnkiError>) -> Value {
    match result {
        Ok(value) => json!({"ok": value}),
        Err(error) => {
            json!({"error": {"kind": match &error { ZjlibCnkiError::Request(_) => "Request", ZjlibCnkiError::Parse(_) => "Parse", ZjlibCnkiError::Timeout(_) => "Timeout"}, "display": error.to_string()}})
        }
    }
}
pub fn observe_migration(request: &Value) -> Value {
    MIGRATION_CLOCK.store(
        request["now"].as_i64().unwrap_or(1800000000),
        std::sync::atomic::Ordering::Relaxed,
    );
    let text = request["text"].as_str().unwrap_or_default();
    match request["kind"].as_str().unwrap() {
        "redirect_wire" => migration_redirect_wire(request),
        "text" => json!({"decoded": decode_html(text), "stripped": strip_tags(text), "clean": clean_text(text), "normalized": normalize_exact_text(text), "filename": safe_filename(text), "authors": split_author_names(text)}),
        "jwt" => json!(jwt_expiration(text).map(|value| value.to_string())),
        "javascript" => json!({"decoded":decode_js_string(text),"sign":extract_js_var(text,"sign"),"url":extract_js_var(text,"url")}),
        "forms" => json!({"result": search_result_form_fields(text), "handler": search_handler_form_fields(text).into_iter().filter(|(name,_)|name!="__").collect::<Vec<_>>()}),
        "location" => migration_outcome(extract_window_location(text,request["base"].as_str().unwrap()).map(|url|json!(url))),
        "sync" => migration_outcome(extract_share_cookie_sync(text,&LiveZjlibCnkiEndpoints::default()).map(|sync|json!(sync.map(|sync|json!({"url":sync.url.to_string(),"fields":sync.fields}))))),
        "cookie" => json!(ZjlibCnkiCookie::from_json(&request["value"]).map(|cookie| json!({"value":cookie.to_json(), "unexpired":cookie.is_unexpired(current_unix_time())}))),
        "identity" => json!(does_article_metadata_match(&migration_identity(&request["expected"]), &migration_identity(&request["actual"]))),
        "mode" => json!(FixtureZjlibCnkiMode::parse(text).map(|mode| format!("{mode:?}"))),
        "html" => {
            let identity=extract_article_identity(text,request["fallback"].as_str().unwrap_or("fallback"));
            json!({"title":identity.title,"authors":identity.authors,"journal_title":identity.journal_title})
        },
        "search" => migration_outcome(parse_search_results(text,request["base"].as_str().unwrap(),&LiveZjlibCnkiEndpoints::default()).map(|results|json!(results.into_iter().map(|result|json!({"index":result.index,"title":result.title,"detail_url":result.detail_url,"file_name":result.file_name,"db_name":result.db_name,"db_code":result.db_code,"download_url":result.download_url})).collect::<Vec<_>>()))),
        "pdf" => migration_outcome(extract_pdf_download_url(text,request["base"].as_str().unwrap(),&LiveZjlibCnkiEndpoints::default()).map(|value|json!(value))),
        "url" => {
            let family=match request["family"].as_str().unwrap(){"Www"=>ZjlibEndpointFamily::Www,"Share"=>ZjlibEndpointFamily::Share,"ZyproxyLogin"=>ZjlibEndpointFamily::ZyproxyLogin,_=>ZjlibEndpointFamily::Zyproxy};
            let endpoints=LiveZjlibCnkiEndpoints::default();
            migration_outcome(if let Some(base)=request["base"].as_str(){join_endpoint_url_for(base,text,family,&endpoints)}else{parse_endpoint_url_for(text,family,&endpoints)}.map(|url|json!(url.to_string())))
        },
        "sequence" => {
            let mode = FixtureZjlibCnkiMode::parse(request["mode"].as_str().unwrap_or("success")).unwrap();
            let mut client = ZhejiangLibraryCnkiClient::from_state_data(FixtureZjlibCnkiTransport::new(mode), &request["state"]);
            let mut observations = Vec::new();
            for operation in request["operations"].as_array().unwrap() {
                if let Some(now) = operation["now"].as_i64() { MIGRATION_CLOCK.store(now, std::sync::atomic::Ordering::Relaxed); }
                let result = match operation["op"].as_str().unwrap() {
                    "load" => {client.load_state_data(&operation["state"]); Ok(Value::Null)},
                    "start" => client.start_qr_login().map(|login| json!({"uuid":login.uuid,"status":login.status,"qr_code":login.qr_code})),
                    "poll" => client.poll_qr_login(operation["timeout"].as_i64().unwrap_or(180), operation["interval"].as_f64().unwrap_or(2.0)).map(|token| json!(token)),
                    "warm" => client.warm_up_fulltext_session().map(|url| json!(url)),
                    "fresh" => Ok(json!(client.has_fresh_fulltext_session(operation["at"].as_i64()))),
                    "download" => client.download_matching_pdf(&migration_identity(&operation["identity"]), operation["limit"].as_u64().unwrap_or(10) as usize).map(|pdf| json!({"filename":pdf.filename,"final_url":pdf.final_url,"content_type":pdf.content_type,"byte_count":pdf.byte_count,"content":String::from_utf8(pdf.content).unwrap()})),
                    "save" => Ok(Value::Null),
                    _ => panic!("unknown operation"),
                };
                observations.push(json!({"result":migration_outcome(result),"state":client.to_state_data()}));
            }
            json!(observations)
        },
        _ => panic!("unknown observation"),
    }
}

fn migration_redirect_wire(request: &Value) -> Value {
    use std::io::{Read, Write};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let location = request["location"]
        .as_str()
        .map(|value| value.replace("{base}", &base));
    let should_redirect = request["redirect"].as_bool().unwrap_or(true);
    let stopped = Arc::new(AtomicBool::new(false));
    let captured = Arc::new(Mutex::new(Vec::<String>::new()));
    let server_stop = stopped.clone();
    let server_requests = captured.clone();
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::Relaxed) {
            let (mut stream, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut byte = [0_u8; 1];
            while !bytes.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap() == 0 {
                    break;
                };
                bytes.push(byte[0]);
            }
            let text = String::from_utf8(bytes).unwrap();
            let mut requests = server_requests.lock().unwrap();
            requests.push(
                text.lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_string(),
            );
            let first = requests.len() == 1;
            drop(requests);
            let body = if should_redirect {
                "%PDF"
            } else {
                r#"{"data":{"uuid":"qr","qrCode":"image","status":"WAITING_SCAN"}}"#
            };
            let header = if first {
                location
                    .as_ref()
                    .map(|value| format!("Location: {value}\r\n"))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let status = if first { 302 } else { 200 };
            let _=stream.write_all(format!("HTTP/1.1 {status} Response\r\nConnection: close\r\nContent-Type: application/pdf\r\nContent-Length: {}\r\n{header}\r\n{body}",body.len()).as_bytes());
        }
    });
    let endpoints = LiveZjlibCnkiEndpoints {
        www_base_url: Url::parse(&format!("{base}/www")).unwrap(),
        share_base_url: Url::parse(&format!("{base}/share")).unwrap(),
        zyproxy_login_base_url: Url::parse(&format!("{base}/login")).unwrap(),
        zyproxy_base_url: Url::parse(&format!("{base}/proxy")).unwrap(),
        entry_url: Url::parse(&format!("{base}/share/entry")).unwrap(),
        library_refer: Url::parse(&format!("{base}/proxy/kns55/")).unwrap(),
    };
    let mut live = LiveZjlibCnkiTransport::new_with_endpoints(
        LiveZjlibCnkiConfig {
            timeout_seconds: 3,
            maximum_document_bytes: 1024,
        },
        endpoints,
        crate::provider_proxy::ProviderProxy::direct(),
        None,
    )
    .unwrap();
    let outcome = if should_redirect {
        live.download_pdf(&format!("{base}/proxy/start"),None,None).map(|pdf|json!({"final":pdf.final_url.strip_prefix(&base).unwrap(),"content":String::from_utf8(pdf.content).unwrap()}))
    } else {
        live.start_qr_login()
            .map(|login| json!({"uuid":login.uuid,"status":login.status,"qr_code":login.qr_code}))
    };
    stopped.store(true, Ordering::Relaxed);
    server.join().unwrap();
    let requests = captured.lock().unwrap().clone();
    json!({"requests":requests,"result":migration_outcome(outcome)})
}

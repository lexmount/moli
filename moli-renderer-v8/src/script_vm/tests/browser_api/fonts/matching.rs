use super::*;

#[test]
fn font_set_queries_distinguish_generic_families_from_literal_names_in_each_realm() {
    let mut vm = new_storage_test_vm("https://font-query-family-kinds.test/");
    let bytes = include_bytes!("../../../../../../moli-layout/tests/fixtures/moli-ahem.ttf");
    vm.eval(&format!(
        r#"(async () => {{
            {}
            const frame = (document.body || document.documentElement || document)
                .appendChild(document.createElement('iframe'));
            const child = frame.contentWindow;
            const bytes = {};
            return {{
                main: await fontFaceSetFamilyQueriesProbe(window, document.fonts, bytes),
                child: await fontFaceSetFamilyQueriesProbe(child, child.document.fonts, bytes),
            }};
        }})().then(
            result => {{ globalThis.familyQueryResult = JSON.stringify(result); }},
            error => {{ globalThis.familyQueryResult = JSON.stringify({{error: String(error.stack || error)}}); }}
        )"#,
        include_str!("../../../../../tests/fixtures/fontfaceset-family-queries.js"),
        serde_json::to_string(bytes.as_slice()).unwrap(),
    ))
    .unwrap();
    let result: serde_json::Value =
        serde_json::from_str(&vm.eval("familyQueryResult").unwrap()).unwrap();
    for realm in ["main", "child"] {
        assert_eq!(result[realm]["failures"], serde_json::json!([]), "{result}");
        assert_eq!(
            result[realm]["rows"].as_array().unwrap().len(),
            69,
            "{result}"
        );
    }
}

#[test]
fn font_set_queries_normalize_reversed_computed_descriptor_ranges() {
    let mut vm = new_storage_test_vm("https://font-query-reversed.test/");
    let url = fixture_font_url();
    vm.eval(&format!(
        r#"
        const bytes=Uint8Array.from(atob('{}'),c=>c.charCodeAt(0));
        globalThis.reversed=[];
        for(const [property,range,single,request] of [
          ['weight','900 100','800','700'],
          ['stretch','125% 75%','120%','expanded'],
          ['style','oblique 30deg 10deg','oblique 20deg','oblique 15deg'],
          ['weight','calc(800 + 100) calc(200 - 100)','800','700'],
          ['stretch','calc(100% + 25%) calc(50% + 25%)','120%','expanded'],
          ['style','oblique calc(15deg + 15deg) calc(5deg + 5deg)','oblique 20deg','oblique 15deg']
        ]) {{
          const family='Reversed'+reversed.length;
          const good=new FontFace(family,bytes,{{[property]:range}});
          const bad=new FontFace(family,'url(data:font/ttf;base64,AA==)',{{[property]:single}});
          bad.loaded.catch(()=>{{}});
          document.fonts.add(good);document.fonts.add(bad);
          const result=[good[property],document.fonts.check(request+' 16px '+family,'A')];
          reversed.push(result);
          document.fonts.load(request+' 16px '+family,'A').then(
            faces=>result.push(faces.length,faces[0]===good,bad.status),
            error=>result.push(error.name));
        }}
    "#,
        url.split_once(',').unwrap().1
    ))
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(reversed)").unwrap(),
        r#"[["900 100",true,1,true,"unloaded"],["125% 75%",true,1,true,"unloaded"],["oblique 30deg 10deg",true,1,true,"unloaded"],["calc(900) calc(100)",true,1,true,"unloaded"],["calc(125%) calc(75%)",true,1,true,"unloaded"],["oblique calc(30deg) calc(10deg)",true,1,true,"unloaded"]]"#
    );
}

#[test]
fn font_set_queries_resolve_css_calculations_in_requests_and_descriptors() {
    let mut completions = RendererResourceCompletionTestHarness::new();
    let mut vm = new_storage_test_vm_with_completion_sender(
        "https://font-query-calculations.test/",
        completions.sender(),
    );
    let url = fixture_font_url();
    vm.eval(&format!(r#"
        const bytes=Uint8Array.from(atob('{}'),c=>c.charCodeAt(0));
        globalThis.calculations=[];
        for (const [family,goodDescriptor,badDescriptor,query] of [
          ['MathWeight',{{weight:'400'}},{{weight:'700'}},'calc(400 + 300) 16px MathWeight'],
          ['MathStyle',{{style:'oblique 14deg'}},{{style:'oblique 20deg'}},'oblique calc(10deg + 10deg) 16px MathStyle'],
          ['MathWidth',{{}},{{stretch:'calc(50% + 25%)'}},'16px MathWidth']
        ]) {{
          const good=new FontFace(family,bytes,goodDescriptor);
          const bad=new FontFace(family,'url(data:font/ttf;base64,AA==)',badDescriptor);
          bad.loaded.catch(()=>{{}});
          document.fonts.add(good);document.fonts.add(bad);
          calculations.push(document.fonts.check(query,'A'));
          document.fonts.load(query,'A').then(
            faces=>calculations.push([faces.length,faces[0]===good,bad.status]),
            error=>calculations.push(error.name));
        }}
    "#,url.split_once(',').unwrap().1)).unwrap();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(
        vm.eval("JSON.stringify(calculations)").unwrap(),
        r#"[false,false,true,[1,true,"unloaded"],"NetworkError","NetworkError"]"#
    );
    vm.eval(
        r#"
        globalThis.widthCalculations = [document.fonts.check('condensed 16px MathWidth','A')];
        document.fonts.load('condensed 16px MathWidth','A').then(
          () => widthCalculations.push('unexpected loaded normal face'),
          error => widthCalculations.push(error.name));
    "#,
    )
    .unwrap();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(
        vm.eval("JSON.stringify(widthCalculations)").unwrap(),
        r#"[false,"NetworkError"]"#
    );
}

#[test]
fn font_set_queries_filter_text_without_loading_unrelated_sources() {
    let mut completions = RendererResourceCompletionTestHarness::new();
    let mut vm = new_storage_test_vm_with_completion_sender(
        "https://font-query-text.test/",
        completions.sender(),
    );
    vm.eval(
        r#"
        globalThis.face = new FontFace('ReviewProbe',
          'url(data:font/ttf;base64,AA==)', {unicodeRange:'U+0041'});
        face.loaded.catch(()=>{});
        document.fonts.add(face);
        globalThis.queryResults = {checks:[], loads:[]};
        for (const text of ['B', '']) {
          queryResults.checks.push(document.fonts.check('16px ReviewProbe',text));
          document.fonts.load('16px ReviewProbe',text).then(
            faces=>queryResults.loads.push([text,faces.length,face.status]),
            error=>queryResults.loads.push([text,error.name,face.status]));
        }
    "#,
    )
    .unwrap();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(
        vm.eval("JSON.stringify({...queryResults,status:face.status})")
            .unwrap(),
        r#"{"checks":[true,true],"loads":[["B",0,"unloaded"],["",0,"unloaded"]],"status":"unloaded"}"#
    );
    vm.eval(
        r#"
        globalThis.matched = [document.fonts.check('16px ReviewProbe','A')];
        document.fonts.load('16px ReviewProbe','A').catch(e=>matched.push(e.name));
    "#,
    )
    .unwrap();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(
        vm.eval("JSON.stringify(matched)").unwrap(),
        r#"[false,"NetworkError"]"#
    );
}

#[test]
fn font_set_queries_select_weight_style_and_stretch_before_loading() {
    let mut completions = RendererResourceCompletionTestHarness::new();
    let mut vm = new_storage_test_vm_with_completion_sender(
        "https://font-query-properties.test/",
        completions.sender(),
    );
    let url = fixture_font_url();
    vm.eval(&format!(
        r#"
        const bytes=Uint8Array.from(atob('{}'),c=>c.charCodeAt(0));
        globalThis.queryResults={{checks:[],loads:[]}};
        globalThis.badFaces=[];
        globalThis.goodFaces=[];
        for (const [family,descriptor] of [
          ['WeightMatch',{{weight:'700'}}],['StyleMatch',{{style:'italic'}}],
          ['WidthMatch',{{stretch:'condensed'}}]
        ]) {{
          const good=new FontFace(family,bytes);
          const bad=new FontFace(family,'url(data:font/ttf;base64,AA==)',descriptor);
          bad.loaded.catch(()=>{{}});
          goodFaces.push(good);badFaces.push(bad);
          document.fonts.add(good);document.fonts.add(bad);
          queryResults.checks.push(document.fonts.check(`16px ${{family}}`,'A'));
          document.fonts.load(`16px ${{family}}`,'A').then(
            faces=>queryResults.loads.push([faces.length,faces[0]===good,bad.status]),
            error=>queryResults.loads.push(error.name));
        }}
    "#,
        url.split_once(',').unwrap().1
    ))
    .unwrap();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(
        vm.eval("JSON.stringify(queryResults)").unwrap(),
        r#"{"checks":[true,true,true],"loads":[[1,true,"unloaded"],[1,true,"unloaded"],[1,true,"unloaded"]]}"#
    );
    vm.eval(r#"
        globalThis.selected = [];
        for (const query of ['bold 16px WeightMatch','italic 16px StyleMatch','condensed 16px WidthMatch']) {
          selected.push(document.fonts.check(query,'A'));
          document.fonts.load(query,'A').catch(e=>selected.push(e.name));
        }
    "#).unwrap();
    complete_local_font_resources(&mut vm, &mut completions);
    assert_eq!(
        vm.eval("JSON.stringify(selected)").unwrap(),
        r#"[false,false,false,"NetworkError","NetworkError","NetworkError"]"#
    );
}

#[test]
fn font_set_queries_select_capabilities_before_unicode_ranges_and_track_descriptor_writes() {
    let mut vm = new_storage_test_vm("https://font-query-order.test/");
    let url = fixture_font_url();
    vm.eval(&format!(
        r#"
        const bytes=Uint8Array.from(atob('{}'),c=>c.charCodeAt(0));
        globalThis.normal=new FontFace('RangeWeight',bytes,{{unicodeRange:'U+0042'}});
        globalThis.bold=new FontFace('RangeWeight','url(data:font/ttf;base64,AA==)',
          {{weight:'700',unicodeRange:'U+0041'}});
        bold.loaded.catch(()=>{{}});
        document.fonts.add(normal);document.fonts.add(bold);
        globalThis.queryResults=[document.fonts.check('16px RangeWeight','A')];
        document.fonts.load('16px RangeWeight','A').then(
          faces=>queryResults.push(faces.length,bold.status),e=>queryResults.push(e.name));
    "#,
        url.split_once(',').unwrap().1
    ))
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(queryResults)").unwrap(),
        r#"[true,0,"unloaded"]"#
    );
    vm.eval(
        r#"
        globalThis.writes=[];
        normal.unicodeRange='U+0041';
        writes.push(document.fonts.check('16px RangeWeight','A'));
        bold.weight='400';
        writes.push(document.fonts.check('16px RangeWeight','A'));
        bold.weight='700';
        writes.push(document.fonts.check('16px RangeWeight','A'));
        writes.push(document.fonts.check('bolder 16px RangeWeight','A'));
    "#,
    )
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(writes)").unwrap(),
        "[true,false,true,false]"
    );
}

#[test]
fn font_set_queries_match_unicode_scalars_and_preserve_family_list_order() {
    let mut vm = new_storage_test_vm("https://font-query-scalars.test/");
    let url = fixture_font_url();
    vm.eval(&format!(
        r#"
        const bytes=Uint8Array.from(atob('{}'),c=>c.charCodeAt(0));
        globalThis.first=new FontFace('FirstQueryFamily',bytes);
        globalThis.second=new FontFace('SecondQueryFamily',bytes);
        const emoji=new FontFace('EmojiQueryFamily','url(data:font/ttf;base64,AA==)',
          {{unicodeRange:'U+1F600'}});
        emoji.loaded.catch(()=>{{}});
        document.fonts.add(first);document.fonts.add(second);document.fonts.add(emoji);
        globalThis.queryResults=[document.fonts.check('16px EmojiQueryFamily','😁'),
          document.fonts.check('16px EmojiQueryFamily','😀')];
        document.fonts.load('16px SecondQueryFamily, FirstQueryFamily','A').then(
          faces=>queryResults.push(faces.length,faces[0]===second,faces[1]===first));
    "#,
        url.split_once(',').unwrap().1
    ))
    .unwrap();
    assert_eq!(
        vm.eval("JSON.stringify(queryResults)").unwrap(),
        "[true,false,2,true,true]"
    );
}

#[test]
fn font_query_generic_families_do_not_match_named_font_faces() {
    for generic in ["serif", "sans-serif", "monospace", "system-ui"] {
        let mut completions = RendererResourceCompletionTestHarness::new();
        let mut vm = new_storage_test_vm_with_completion_sender(
            "https://font-matching.test/",
            completions.sender(),
        );
        vm.eval(&format!(r#"
            globalThis.face=new FontFace('{generic}','url(data:font/ttf;base64,AA==)');
            face.loaded.catch(()=>{{}});document.fonts.add(face);
            globalThis.results=[];
            document.fonts.load('16px {generic}','A').then(faces=>results.push(faces.length),error=>results.push(error.name));
        "#)).unwrap();
        assert_eq!(
            vm.eval(&format!("document.fonts.check('16px {generic}','A')"))
                .unwrap(),
            "true",
            "{generic}"
        );
        assert_eq!(
            vm.eval("JSON.stringify(results)").unwrap(),
            "[0]",
            "{generic}"
        );
        assert_eq!(vm.eval("face.status").unwrap(), "unloaded", "{generic}");
        vm.eval(&format!(r#"document.fonts.load('16px "{generic}"','A').then(faces=>results.push(faces.length),error=>results.push(error.name))"#)).unwrap();
        complete_local_font_resources(&mut vm, &mut completions);
        assert_eq!(
            vm.eval("JSON.stringify(results)").unwrap(),
            r#"[0,"NetworkError"]"#
        );
        assert_eq!(vm.eval("face.status").unwrap(), "error");
    }
}

#[test]
fn font_query_matches_chromium_unicode_case_folding() {
    for (family, query, matches) in [
        ("Äpfel", "äpfel", true),
        ("Σπίτι", "σπίτι", true),
        ("ΟΣ", "ος", true),
        ("K", "k", true),
        ("ẞ", "ß", true),
        ("µ", "Μ", true),
        ("Straße", "STRASSE", false),
        ("Å", "A\u{030a}", false),
        ("İ", "i", false),
        ("İ", "i\u{0307}", false),
        ("𐐀", "𐐨", false),
    ] {
        let mut completions = RendererResourceCompletionTestHarness::new();
        let mut vm = new_storage_test_vm_with_completion_sender(
            "https://unicode-font-family.test/",
            completions.sender(),
        );
        vm.eval(&format!(
            r#"
            globalThis.face=new FontFace('{family}','url(data:font/ttf;base64,AA==)');
            face.loaded.catch(()=>{{}});document.fonts.add(face);
            globalThis.result=[];
        "#
        ))
        .unwrap();
        let font = format!("16px \"{query}\"");
        assert_eq!(
            vm.eval(&format!("document.fonts.check('{font}','A')"))
                .unwrap(),
            (!matches).to_string(),
            "{family} / {query}"
        );
        vm.eval(&format!("document.fonts.load('{font}','A').then(faces=>result.push(faces.length),error=>result.push(error.name))")).unwrap();
        complete_local_font_resources(&mut vm, &mut completions);
        assert_eq!(
            vm.eval("JSON.stringify(result)").unwrap(),
            if matches {
                r#"["NetworkError"]"#
            } else {
                "[0]"
            },
            "{family} / {query}"
        );
        assert_eq!(
            vm.eval("face.status").unwrap(),
            if matches { "error" } else { "unloaded" }
        );
    }
}

#[test]
fn unicode_font_family_queries_and_layout_use_the_same_matching_key() {
    let (narrow, wide) = super::selection_order::order_font_data();
    for (family, query, matches) in [
        ("Äpfel", "äpfel", true),
        ("Σπίτι", "σπίτι", true),
        ("ΟΣ", "ος", true),
        ("K", "k", true),
        ("ẞ", "ß", true),
        ("µ", "Μ", true),
        ("Straße", "STRASSE", false),
        ("Å", "A\u{030a}", false),
        ("İ", "i", false),
        ("𐐀", "𐐨", false),
    ] {
        let mut vm = super::selection_order::order_vm();
        vm.eval(&format!(
            r#"
            const bytes=data=>Uint8Array.from(atob(data),c=>c.charCodeAt(0));
            globalThis.target=new FontFace('{family}',bytes('{wide}'));
            document.fonts.add(target);
            document.fonts.add(new FontFace('FallbackFixture',bytes('{narrow}')));
            probe.style.fontFamily='"{query}", FallbackFixture';
            globalThis.matches=false;
            document.fonts.load('40px "{query}"','A').then(faces=>{{matches=faces[0]===target}});
        "#
        ))
        .unwrap();
        assert_eq!(
            vm.eval("matches").unwrap(),
            matches.to_string(),
            "{family} / {query}"
        );
        assert_eq!(
            vm.eval("String(probe.getBoundingClientRect().width)")
                .unwrap(),
            if matches { "160" } else { "80" },
            "{family} / {query}"
        );
    }
}

use effect_cms::{definitions::*, temporal::*, localization::*};
use std::sync::Arc;

#[test]
fn definition_builders_preserve_order_metadata_and_unified_routes() {
    let featured = new_featured_resource_list_definition(vec![ResourceListItemMetaDefinition {label:"author".into(), ..Default::default()}]);
    assert_eq!(featured.layout.as_str(), RESOURCE_LIST_LAYOUT_FEATURED);
    assert_eq!(featured.sections.to_vec().iter().map(|s|s.key.as_str()).collect::<Vec<_>>(), [RESOURCE_SECTION_KEY_FEATURED,RESOURCE_SECTION_KEY_SECONDARY,RESOURCE_SECTION_KEY_GRID]);
    assert_eq!(featured.item.meta.to_vec()[0].label,"author");
    let results = new_results_resource_list_definition(vec![]);
    assert_eq!(results.layout.as_str(),RESOURCE_LIST_LAYOUT_RESULTS);
    assert_eq!(results.sections.to_vec()[0].key.as_str(),RESOURCE_SECTION_KEY_RESULTS);
    let section = new_titled_resource_form_section(ResourceSectionKey::new(RESOURCE_SECTION_KEY_LOCALIZED_CONTENT),"title.key".into(),"description.key".into(),vec![ResourceFormFieldDefinition {name:"content".into(),..Default::default()}]);
    assert_eq!((section.title_key.as_str(),section.description_key.as_str(),section.fields.to_vec()[0].name.as_str()),("title.key","description.key","content"));
    let normalized = normalize_resource_definition(ResourceDefinition {management_route_mode:ResourceManagementRouteMode::new(RESOURCE_MANAGEMENT_ROUTE_MODE_UNIFIED),management_routes:ResourceRouteSet {dashboard:"manage".into(),..Default::default()},..Default::default()});
    assert_eq!(normalized.management_routes.list,"manage");
}

#[test]
fn registry_retains_nested_aliases_but_copies_outer_values_and_duplicate_order() {
    let filters = SharedList::new(vec![ResourceFilterDefinition {key:ResourceFilterKey::new(RESOURCE_FILTER_STATUS),param:"initial".into(),..Default::default()}]);
    let routes = SharedMap::default();
    routes.insert(ResourceAuxRoute::new("edit"),"initial".to_owned());
    let registry = new_definition_registry(vec![ResourceDefinition {kind:ResourceKind::new("sample"),filters:filters.clone(),management_routes:ResourceRouteSet {aux:routes.clone(),..Default::default()},..Default::default()}]);
    assert!(filters.with_element_mut(0,|f| f.param="changed-input".into()));
    routes.insert(ResourceAuxRoute::new("edit"),"changed-input".into());
    let got = registry.definition(&ResourceKind::new("sample")).unwrap();
    assert_eq!(got.filters.to_vec()[0].param,"changed-input");
    assert_eq!(got.management_routes.aux.get(&ResourceAuxRoute::new("edit")).unwrap(),"changed-input");
    assert!(got.filters.with_element_mut(0,|f| f.param="changed-result".into()));
    got.management_routes.aux.insert(ResourceAuxRoute::new("edit"),"changed-result".into());
    assert_eq!(filters.to_vec()[0].param,"changed-result");
    assert_eq!(routes.get(&ResourceAuxRoute::new("edit")).unwrap(),"changed-result");
    assert!(registry.definition(&ResourceKind::new("absent")).is_err());
    let registry = new_definition_registry(vec![ResourceDefinition {kind:ResourceKind::new("sample"),..Default::default()},ResourceDefinition {kind:ResourceKind::new("other"),..Default::default()},ResourceDefinition {kind:ResourceKind::new("sample"),management_route_mode:ResourceManagementRouteMode::new(RESOURCE_MANAGEMENT_ROUTE_MODE_UNIFIED),management_routes:ResourceRouteSet {dashboard:"new".into(),..Default::default()},..Default::default()}]);
    let mut first=registry.definitions();
    assert_eq!(first.iter().map(|d|d.kind.as_str()).collect::<Vec<_>>(),["sample","other","sample"]);
    assert_eq!(first[0].management_routes.list,"new");
    first[0].kind=ResourceKind::new("changed");
    assert_eq!(registry.definitions()[0].kind.as_str(),"sample");
}

#[test]
fn shared_list_preserves_view_lengths_and_reallocation_independence() {
    let mut storage=Vec::with_capacity(4); storage.extend([1,2]);
    let mut first=SharedList::wrap(storage);
    let mut second=first.clone();
    first.push(3);
    assert_eq!(second.len(),2);
    second.push(9);
    assert_eq!(first.to_vec(),[1,2,9]);
    let old=first.clone();
    first.push(4); first.push(5);
    assert!(first.with_element_mut(0,|v|*v=8));
    assert_eq!(old.to_vec(),[1,2,9]);
    assert_eq!(first.to_vec(),[8,2,9,4,5]);
}

#[test]
fn datetime_classification_translation_and_dst_use_declared_zone() {
    for (date,zone,code) in [("","America/Toronto",DATE_TIME_VALIDATION_REQUIRED),("2026-04-01T18:00:00","Not/AZone",DATE_TIME_VALIDATION_INVALID_TIME_ZONE),("bad","UTC",DATE_TIME_VALIDATION_INVALID_DATE_TIME)] {
        let got=normalize_date_time_form_values(date,zone); assert_eq!(got.2,code); assert!(got.0.is_empty()&&got.1.is_empty());
    }
    assert_eq!(normalize_date_time_form_values("2026-04-01T18:00:00","America/Toronto"),("2026-04-01T22:00:00Z".into(),"America/Toronto".into(),DATE_TIME_VALIDATION_VALID));
    assert_eq!(normalize_date_time_form_values("2026-01-01T18:00:00","America/Toronto").0,"2026-01-01T23:00:00Z");
    let keys=DateTimeValidationMessageKeys {required:"datetime.required".into(),invalid_date_time:"datetime.invalid_date_time".into(),invalid_time_zone:"datetime.invalid_time_zone".into()};
    let translate=|key:&str|format!("translated:{key}");
    assert_eq!(normalize_date_time_form_values_with_translated_message(Some(&translate),"","America/Toronto",&keys).2,"translated:datetime.required");
    assert_eq!(normalize_date_time_form_values_with_translated_message(Some(&translate),"bad","Not/AZone",&keys).2,"translated:datetime.invalid_time_zone");
    assert_eq!(normalize_date_time_form_values_with_translated_message(Some(&translate),"","UTC",&DateTimeValidationMessageKeys::default()).2,"");
    let local=local_date_in_time_zone("2026-05-01T01:30:00Z","America/Toronto").unwrap();
    assert_eq!((local.year(),local.month(),local.day(),local.hour(),local.minute()),(2026,4,30,21,30));
    let now=TemporalInstant::new(chrono::DateTime::parse_from_rfc3339("2026-05-01T12:00:00Z").unwrap().with_timezone(&chrono::Utc),TemporalZone::iana("UTC").unwrap());
    for (date,want) in [("2026-05-01T14:00:00Z",TEMPORAL_STATUS_TODAY),("2026-05-02T14:00:00Z",TEMPORAL_STATUS_UPCOMING),("2026-04-30T14:00:00Z",TEMPORAL_STATUS_PAST)] { assert_eq!(temporal_status_at(date,"America/Toronto",&now),Some(want)); }
}

struct French;
impl TemporalContext for French {
    fn translate(&self,key:&str,args:&[FormatArg<'_>])->String {
        match (key,args) {
            ("time.date_format",[FormatArg::Int(day),FormatArg::Text(month),FormatArg::Int(year)])=>format!("{day} {month} {year}"),
            ("time.date_time_format",[FormatArg::Text(date),FormatArg::Text(time)])=>format!("{date} à {time}"),
            ("time.with_time_zone",[FormatArg::Text(date),FormatArg::Text(zone)])=>format!("{date} · {zone}"),
            ("time.time_format",_)=>"15:04".into(),
            ("component.datetimepicker.month.march",_)=>"mars".into(),
            (k,_) if k.starts_with("component.durationpicker.")=>match k.trim_start_matches("component.durationpicker.") {"week_singular"=>"semaine","week_plural"=>"semaines","day_singular"=>"jour","day_plural"=>"jours","hour_singular"=>"heure","hour_plural"=>"heures","minute_singular"=>"minute","minute_plural"=>"minutes","part_joiner"=>"space",_=>key}.into(),
            _=>key.into(),
        }
    }
}
#[test]
fn temporal_localized_formats_and_duration_precedence_match_source() {
    assert_eq!(format_localized_date_time_ctx(&French,"2026-03-18T13:35:00Z","America/Toronto"),"18 mars 2026 à 09:35 · EDT");
    for (canonical,text,w,d,h,m,want,ok) in [("","2 semaines 3 jours 4 heures","","","","","24720",true),("","","2","3","4","30","24750",true),("90","efdsgfsg","0","0","0","0","",false),("24750","","","","","","24750",true)] { assert_eq!(normalize_duration_form_values(canonical,text,w,d,h,m),(want.into(),ok)); }
    assert_eq!(format_duration(&French,"90"),"1 heure 30 minutes");
    assert_eq!(format_duration(&French,"1.5h"),"1 heure 30 minutes");
    assert_eq!(format_duration(&French,"24750"),"2 semaines 3 jours 4 heures 30 minutes");
}

#[test]
fn language_normalization_preserves_configured_locales_and_field_suffixes() {
    let normalize=new_contract_language_normalizer(ContractLanguageNormalizerOptions { default_language:"en".into(),preserve:["en","en-us","fr","fr-ca","es","pt-br"].map(str::to_owned).to_vec(),normalize:Some(Arc::new(|s|s.trim().to_lowercase())),fallback_key:Some(Arc::new(|s|s.split('-').next().unwrap_or(s).into())) });
    for (input,want) in [("","en"),(" en-US ","en-us"),("fr-CA","fr-ca"),("es-MX","es"),("pt-BR","pt-br"),("de-DE","de")] {assert_eq!(normalize(input),want);}
    assert_eq!(localized_field_name(" content ","fr-CA",Some(normalize.as_ref())),"content_fr_ca");
}

#[test]
fn requested_duration_keeps_canonical_or_raw_fallback_from_current_decoded_form() {
    use std::collections::BTreeMap;
    let valid=BTreeMap::from([(b"duration_text".to_vec(),vec![b"2h".to_vec()])]);
    let values=DurationFormValues::new(Some(&valid),None);
    assert_eq!(requested_duration_form_value_from_decoded(Some(&values)),"120");
    let invalid=BTreeMap::from([(b"duration".to_vec(),vec![b"PT30M".to_vec()]),(b"duration_text".to_vec(),vec![b"not-a-duration".to_vec()])]);
    let values=DurationFormValues::new(Some(&invalid),None);
    assert_eq!(requested_duration_form_value_from_decoded(Some(&values)),"PT30M");
    assert_eq!(invalid_duration_text_from_decoded(Some(&values)),"not-a-duration");
}

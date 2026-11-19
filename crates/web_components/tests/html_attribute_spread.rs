use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use web_components::html;

#[test]
fn spreads_preserve_attribute_order_and_evaluate_once() {
    let events = RefCell::new(Vec::new());
    let record = |event| {
        events.borrow_mut().push(event);
        event
    };

    let rendered = html! {
        div data-first=(record("first"))
            ..({
                record("spread one");
                [("data-first", "duplicate"), ("data-on:click", "<&\"' >")]
                    .into_iter()
                    .inspect(|_| { record("spread one item"); })
            })
            #card .active
            title=(record("middle"))
            ..({
                record("spread two");
                [("data-empty", ""), ("data-amp&name", "kept")]
            })
            data-last=(record("last"))
        {
            (record("child"))
        }
    };

    assert_eq!(
        rendered.into_string(),
        concat!(
            "<div class=\"active\" id=\"card\" data-first=\"first\" data-first=\"duplicate\" ",
            "data-on:click=\"&lt;&amp;&quot;' &gt;\" title=\"middle\" data-empty=\"\" ",
            "data-amp&amp;name=\"kept\" data-last=\"last\">child</div>",
        ),
    );
    assert_eq!(
        events.into_inner(),
        [
            "first",
            "spread one",
            "spread one item",
            "spread one item",
            "middle",
            "spread two",
            "last",
            "child"
        ],
    );
}

#[test]
fn spreads_work_on_nested_and_void_elements_in_template_control_flow() {
    let section_attrs = BTreeMap::from([(String::from("data-section"), String::from("menu"))]);
    let child_calls = Cell::new(0);
    let child = || {
        child_calls.set(child_calls.get() + 1);
        "<aside>"
    };

    let rendered = html! {
        section ..(&section_attrs) {
            @for (name, show) in [("shown", true), ("hidden", false)] {
                @if show {
                    input type="text" ..([("name", name), ("value", "\"><aside>")]) required;
                    label ..([("for", name)]) { (child()) }
                } @else {
                    span ..([("data-skipped", name)]) {}
                }
            }
        }
    };

    assert_eq!(
        rendered.into_string(),
        concat!(
            "<section data-section=\"menu\">",
            "<input type=\"text\" name=\"shown\" value=\"&quot;&gt;&lt;aside&gt;\" required>",
            "<label for=\"shown\">&lt;aside&gt;</label><span data-skipped=\"hidden\"></span></section>",
        ),
    );
    assert_eq!(child_calls.get(), 1);
}

#[test]
fn spreads_in_match_arms_preserve_ranges_and_utf8_markup() {
    let limit = 2;
    let empty = [] as [(&str, &str); 0];

    let rendered = html! {
        "Café 茶"
        @for index in 0..(limit) {
            @match index {
                0 => { p title="été" ..(empty) ..(empty) ..([("lang", "fr")]) { "début" } }
                _ => { p ..([("lang", "ja")]) ..(empty) ..(empty) { "終わり" } }
            }
        }
    };

    assert_eq!(
        rendered.into_string(),
        "Café 茶<p title=\"été\" lang=\"fr\">début</p><p lang=\"ja\">終わり</p>",
    );
    assert_eq!(
        html! { ..([("lang", "fr")]).message { "salut" } }.into_string(),
        "<div class=\"message\" lang=\"fr\">salut</div>",
    );
}

#[test]
fn invalid_attribute_names_stop_before_later_attributes_or_children() {
    for name in [
        "",
        "has space",
        "has\tspace",
        "has\u{00a0}space",
        "has\u{007f}control",
        "has'apostrophe",
        "has\"quote",
        "has<opening",
        "has>closing",
        "has/slash",
        "has=equals",
    ] {
        let later_attribute = Cell::new(false);
        let child = Cell::new(false);
        let rendered = catch_unwind(AssertUnwindSafe(|| {
            html! {
                div ..([(name, "value")]) title=({ later_attribute.set(true); "later" }) {
                    ({ child.set(true); "child" })
                }
            }
        }));

        assert!(
            rendered.is_err(),
            "accepted invalid attribute name {name:?}"
        );
        assert!(
            !later_attribute.get(),
            "evaluated a later attribute for {name:?}"
        );
        assert!(!child.get(), "evaluated a child for {name:?}");
    }
}

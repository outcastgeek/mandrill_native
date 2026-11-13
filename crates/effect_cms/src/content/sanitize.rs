//! The content sanitation policy and its sanitizer.
//!
//! Author-supplied HTML is filtered by a usage-oriented policy: `class` matching a
//! paragraph-safe pattern globally, `id` matching `^[A-Za-z][A-Za-z0-9_\-:.]*$` on
//! `h1`–`h6`, `style` on `span`, `code` and `pre`, and data attributes. This module
//! owns that policy as explicit data and applies it with token dispatch, attribute
//! filtering, URL validation and `rel="nofollow"` handling.
//!
//! # Policy behavior
//!
//! - The element/attribute allow list covers standard attributes, URLs, lists,
//!   tables and images plus the four additions above.
//! - Attribute values use explicit value rules. The `nowrap` rule is intentionally
//!   match-always (`(?i)|nowrap` has an empty alternative), and that behavior is kept.
//! - Attribute policies are element-specific first and global second, so a heading
//!   `id` that fails the heading rule still passes through the global `id` rule.
//! - A start tag whose attributes are all filtered out is dropped — including its
//!   closing tag — unless the element is in the seeded valid-without-attributes set.
//!   That set, not the allow list, decides this: `a`, `img` and `meter` are dropped
//!   without a surviving attribute while `tr`, `td` and `q` are not. Its text content
//!   is kept.
//! - Disallowed start/end tags are dropped; their text content is kept and escaped
//!   unless the element is in the skip-content set (`frame`, `frameset`, `iframe`,
//!   `noembed`, `noframes`, `noscript`, `nostyle`, `object`, `script`, `style`,
//!   `title`), which suppresses character content while open. Comments and doctypes
//!   are dropped; `script` and `style` tags and their raw text are dropped.
//! - Linkable elements have their URL attributes validated and replaced by the
//!   validated URL: `a`/`area`/`base`/`link` `href`, `blockquote`/`del`/`ins`/`q`
//!   `cite`, and `audio`/`embed`/`iframe`/`img`/`script`/`source`/`track`/`video`
//!   `src`. A `data:` value is never accepted because the policy allows only
//!   `mailto`, `http` and `https`.
//! - `rel="nofollow"` is appended to a link element that carries an `href`, and an
//!   existing `rel` value gains ` nofollow` when it does not already contain it.
//! - The surviving token stream is written by `html5ever`'s HTML serializer, not by
//!   this module: element delimiters, attribute values and character data are the
//!   serializer's output, so no tag, attribute or text is assembled here. Element
//!   names are normalized (`QuoteToASCII` then lowercasing) before the
//!   `script`/`style` guard.
//! - Raw-text tokenizer states are selected from a tag-name table: `script` is
//!   script data, `style`, `iframe`, `noembed`, `noframes`, `noscript` and `xmp` are
//!   raw text, `textarea` and `title` are RCDATA, and `plaintext` is the plaintext
//!   state. Raw text therefore never becomes active markup.
//! - No style policies are declared, so a `style` attribute is an ordinary allowed
//!   attribute value and no CSS parsing is involved.
//!
//! # Native parser and serializer boundaries
//!
//! URL components are validated and encoded with the source policy: allowed scheme
//! names normalize to lowercase, authority/port/IPv6 syntax is checked, valid raw
//! path/fragment escapes remain intact, and opaque data and raw queries stay raw.
//! No WHATWG host/path rewriting or new URL scheme policy is introduced.
//!
//! - **Serializer spelling.** The filtered content is serialized by `html5ever`, so
//!   its exact bytes differ from a token-preserving renderer where the two spell the
//!   same content differently: a self-closing non-void element is written
//!   `<name></name>` (the serializer has no self-closing form) and a self-closing void
//!   element `<name>` rather than `<name/>`; `"` in an attribute value is `&quot;` and
//!   a non-breaking space is `&nbsp;`; `'`, `"` and `\r` in character data are not
//!   escaped. Filtering decisions, attribute selection and text suppression are
//!   unaffected.
//! - **Tokenizer diagnostics.** HTML is tokenized with a WHATWG tokenizer, so
//!   character references, raw-text elements and error recovery do not need a
//!   hand-written replacement. Differences on malformed input are not enumerated.
//! - **NUL character data.** Default-data NUL tokens remain NUL character data;
//!   RCDATA/raw-text states replace NUL with U+FFFD in the native tokenizer,
//!   matching the source tokenizer's state-specific conversion.
//!
//! These native representation differences are disclosed; exact malformed-token
//! or serializer byte identity is not claimed and no renderer fork is introduced.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::LazyLock;

use html5ever::serialize::{HtmlSerializer, SerializeOpts, Serializer, TraversalScope};
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use html5ever::{LocalName, QualName, ns};
use regex::Regex;

/// The content policy value, built once.
///
/// An immutable, compiled allow list. It holds no service, provider or mutable state,
/// so it is a constant rather than a service locator.
static CONTENT_SANITIZE_POLICY: LazyLock<SanitizePolicy> =
    LazyLock::new(build_content_sanitize_policy);

/// One attribute value rule.
#[derive(Clone, Debug)]
enum AttributeValueRule {
    /// Any value is allowed.
    Any,
    /// The value must match the pattern with unanchored search.
    Pattern(Regex),
    /// No value is allowed; used only when a literal pattern cannot be compiled.
    Deny,
}

impl AttributeValueRule {
    /// Whether this rule allows `value`.
    fn matches(&self, value: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Pattern(regex) => regex.is_match(value),
            Self::Deny => false,
        }
    }
}

/// The content sanitation policy.
#[derive(Debug)]
struct SanitizePolicy {
    /// Element-specific attribute rules.
    element_attributes: BTreeMap<&'static str, BTreeMap<&'static str, Vec<AttributeValueRule>>>,
    /// Elements the policy allows.
    allowed_elements: BTreeSet<&'static str>,
    /// Attribute rules that apply to every allowed element.
    global_attributes: BTreeMap<&'static str, Vec<AttributeValueRule>>,
    /// Whether `data-*` attributes are allowed.
    allow_data_attributes: bool,
    /// Whether URLs must be parseable and allowed by the URL policy.
    require_parseable_urls: bool,
    /// Whether relative URLs are allowed.
    allow_relative_urls: bool,
    /// The allowed URL schemes.
    allowed_url_schemes: &'static [&'static str],
    /// Whether link elements gain `rel="nofollow"`.
    require_nofollow_on_links: bool,
}

impl SanitizePolicy {
    /// An empty policy with the seeded element sets.
    fn new() -> Self {
        Self {
            element_attributes: BTreeMap::new(),
            allowed_elements: BTreeSet::new(),
            global_attributes: BTreeMap::new(),
            allow_data_attributes: false,
            require_parseable_urls: false,
            allow_relative_urls: false,
            allowed_url_schemes: &[],
            require_nofollow_on_links: false,
        }
    }

    /// Allow these elements with no attributes of their own.
    fn allow_elements(&mut self, names: &[&'static str]) {
        for name in names {
            self.allowed_elements.insert(*name);
        }
    }

    /// Allow attributes on specific elements.
    fn allow_element_attributes(
        &mut self,
        elements: &[&'static str],
        attributes: &[&'static str],
        rule: AttributeValueRule,
    ) {
        for element in elements {
            self.allowed_elements.insert(*element);
            let entry = self.element_attributes.entry(*element).or_default();
            for attribute in attributes {
                entry.entry(*attribute).or_default().push(rule.clone());
            }
        }
    }

    /// Allow attributes on every element.
    fn allow_global_attributes(&mut self, attributes: &[&'static str], rule: AttributeValueRule) {
        for attribute in attributes {
            self.global_attributes
                .entry(*attribute)
                .or_default()
                .push(rule.clone());
        }
    }

    /// Whether the policy allows this element name.
    ///
    /// This policy declares no element-name regexes, so an element is allowed
    /// exactly when an element or attribute rule registered it.
    fn allows_element(&self, element: &str) -> bool {
        self.allowed_elements.contains(element)
    }

    /// Whether the policy emits this element when no attribute survived.
    ///
    /// The policy seeds [`ELEMENTS_WITHOUT_ATTRIBUTES`] with the elements
    /// it knows are valid without any attribute; an allowed element outside that set
    /// is dropped when its attributes are all filtered out.
    fn allows_element_without_attributes(&self, element: &str) -> bool {
        ELEMENTS_WITHOUT_ATTRIBUTES.contains(&element)
    }

    /// Whether one element-specific attribute rule allows the value.
    fn element_rule_matches(&self, element: &str, attribute: &str, value: &str) -> bool {
        let Some(attributes) = self.element_attributes.get(element) else {
            return false;
        };
        let Some(rules) = attributes.get(attribute) else {
            return false;
        };
        rules.iter().any(|rule| rule.matches(value))
    }

    /// Whether one global attribute rule allows the value.
    fn global_rule_matches(&self, attribute: &str, value: &str) -> bool {
        let Some(rules) = self.global_attributes.get(attribute) else {
            return false;
        };
        rules.iter().any(|rule| rule.matches(value))
    }

    /// Filter one element's attributes.
    ///
    /// Order: data attributes, then element rules, then
    /// global rules; an empty result returns before the URL policy; otherwise URL
    /// attributes are validated and `rel="nofollow"` is applied to link elements.
    fn sanitize_attributes(
        &self,
        element: &str,
        attrs: &[html5ever::Attribute],
    ) -> CleanAttributes {
        let mut clean = CleanAttributes::default();
        for attribute in attrs {
            let key = attribute_key(attribute);
            let value: &str = &attribute.value;
            if self.allow_data_attributes && is_data_attribute(&key) {
                clean.push(key, value);
                continue;
            }
            if self.element_rule_matches(element, &key, value) {
                clean.push(key, value);
                continue;
            }
            if self.global_rule_matches(&key, value) {
                clean.push(key, value);
                continue;
            }
        }
        if !self.require_parseable_urls || clean.is_empty() {
            return clean;
        }
        clean.retain_valid_urls(self, element);
        if self.require_nofollow_on_links && is_link_element(element) {
            clean.add_nofollow();
        }
        clean
    }

    /// Validate one URL value against the policy.
    ///
    /// A value with a space, tab or newline is rejected: a `data:`
    /// rescue path exists only to make a data URI parseable, and this policy allows
    /// no `data` scheme, so it can never produce an accepted result here.
    fn sanitize_url(&self, raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.contains(' ') || trimmed.contains('\t') || trimmed.contains('\n') {
            return None;
        }
        if has_control_characters(trimmed) {
            return None;
        }
        let scheme = parse_scheme(trimmed)?;
        if let Some(scheme) = &scheme {
            if !self
                .allowed_url_schemes
                .iter()
                .any(|allowed| *allowed == scheme)
            {
                return None;
            }
        } else if !self.allow_relative_urls {
            return None;
        }
        let encoded = encode_content_url(trimmed, scheme.as_deref())?;
        (!encoded.is_empty()).then_some(encoded)
    }
}

/// The allowed attributes of one element, in their surviving source order.
#[derive(Debug, Default)]
struct CleanAttributes {
    /// The attribute name/value pairs.
    entries: Vec<(String, String)>,
}

impl CleanAttributes {
    /// Append one surviving attribute.
    fn push(&mut self, key: String, value: &str) {
        self.entries.push((key, value.to_owned()));
    }

    /// Whether no attribute survived.
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether an attribute with this name survived.
    fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(name, _)| name == key)
    }

    /// Drop or rewrite the URL attributes of a linkable element.
    fn retain_valid_urls(&mut self, policy: &SanitizePolicy, element: &str) {
        let mut validated: Vec<(String, String)> = Vec::with_capacity(self.entries.len());
        for (key, value) in &self.entries {
            if is_url_attribute(element, key) {
                if let Some(url) = policy.sanitize_url(value) {
                    validated.push((key.clone(), url));
                }
            } else {
                validated.push((key.clone(), value.clone()));
            }
        }
        self.entries = validated;
    }

    /// Apply the `rel="nofollow"` rule for link elements with an `href`.
    fn add_nofollow(&mut self) {
        if !self.contains("href") {
            return;
        }
        let mut nofollow_found = false;
        for (key, value) in &mut self.entries {
            if key == "rel" {
                if !value.contains("nofollow") {
                    value.push_str(" nofollow");
                }
                nofollow_found = true;
            }
        }
        if !nofollow_found {
            self.entries.push(("rel".to_owned(), "nofollow".to_owned()));
        }
    }
}

/// The sanitizer state for one document.
struct SanitizeState<'policy> {
    /// The policy being applied.
    policy: &'policy SanitizePolicy,
    /// The serializer that writes the surviving tokens.
    ///
    /// The surviving tokens are written by the HTML serializer for the
    /// processed-content boundary. No markup is assembled by this module.
    serializer: HtmlSerializer<Vec<u8>>,
    /// Whether an unclosed dropped start tag still has to swallow its end tag.
    skip_closing_tag: bool,
    /// The dropped start tags whose end tags are swallowed.
    closing_tags_to_skip: Vec<String>,
    /// The most recently started element name, used for the raw-text guard.
    most_recently_started_token: String,
    /// Whether a dropped skip-content element is currently suppressing its content.
    skip_element_content: bool,
    /// How many dropped skip-content elements are currently open.
    skipping_elements_count: i64,
}

impl<'policy> SanitizeState<'policy> {
    /// A fresh state with no output.
    fn new(policy: &'policy SanitizePolicy) -> Self {
        let options = SerializeOpts {
            // The filtered token stream is not a tree: a stray end tag must be
            // written, not panic, and it must not shift a real element's parent.
            create_missing_parent: true,
            traversal_scope: TraversalScope::ChildrenOnly(None),
            ..SerializeOpts::default()
        };
        Self {
            policy,
            serializer: HtmlSerializer::new(Vec::new(), options),
            skip_closing_tag: false,
            closing_tags_to_skip: Vec::new(),
            most_recently_started_token: String::new(),
            skip_element_content: false,
            skipping_elements_count: 0,
        }
    }

    /// Apply the policy to one token.
    fn process(&mut self, token: &Token) {
        match token {
            // Doctypes have no safe parsing mechanism and are dropped; comments are
            // dropped unless a policy allows them, and this policy does not.
            Token::DoctypeToken(_) | Token::CommentToken(_) => {}
            Token::TagToken(tag) => self.process_tag(tag),
            Token::CharacterTokens(data) => self.write_text(data),
            Token::NullCharacterToken => self.write_text("\u{0}"),
            Token::EOFToken | Token::ParseError(_) => {}
        }
    }

    /// Write character data through the serializer unless raw or skipped content is
    /// suppressed.
    fn write_text(&mut self, data: &str) {
        if self.skip_element_content || is_unsafe_element(&self.most_recently_started_token) {
            return;
        }
        let _ = self.serializer.write_text(data);
    }

    /// Write one surviving start tag through the serializer.
    ///
    /// A void element is closed immediately: the serializer keeps an element stack
    /// for its child-suppression rule, and html5ever exposes no token-stream mode,
    /// so leaving a void element open would suppress the elements that follow it.
    /// A non-void self-closing token keeps its open element, because HTML parsing
    /// ignores the slash and the token is emitted unchanged.
    fn write_start_tag(&mut self, name: &str, attributes: &CleanAttributes) {
        let element = qualified_element_name(name);
        let attribute_values: Vec<(QualName, &str)> = attributes
            .entries
            .iter()
            .map(|(key, value)| {
                let name = QualName::new(None, ns!(), LocalName::from(key.as_str()));
                (name, value.as_str())
            })
            .collect();
        let _ = self.serializer.start_elem(
            element.clone(),
            attribute_values.iter().map(|(name, value)| (name, *value)),
        );
        if is_void_element(name) {
            let _ = self.serializer.end_elem(element);
        }
    }

    /// Write one surviving end tag through the serializer.
    fn write_end_tag(&mut self, name: &str) {
        let _ = self.serializer.end_elem(qualified_element_name(name));
    }

    /// Take the serializer's output as text.
    fn take_output(&mut self) -> String {
        let bytes = std::mem::take(&mut self.serializer.writer);
        match String::from_utf8(bytes) {
            Ok(output) => output,
            // The serializer writes the UTF-8 text it is given plus ASCII markup, so
            // this branch only guards the boundary against an invalid byte sequence.
            Err(error) => {
                let bytes = error.into_bytes();
                String::from_utf8_lossy(&bytes).into_owned()
            }
        }
    }

    /// Apply the policy to one tag token.
    fn process_tag(&mut self, tag: &Tag) {
        let name: &str = tag.name.as_ref();
        if tag.kind == TagKind::EndTag {
            self.process_end_tag(name);
            return;
        }
        if tag.self_closing {
            self.process_self_closing_tag(name, tag);
            return;
        }

        self.most_recently_started_token = normalise_element_name(name);
        if is_unsafe_element(&self.most_recently_started_token) {
            return;
        }
        if !self.policy.allows_element(name) {
            if ELEMENTS_SKIPPING_CONTENT.contains(&name) {
                self.skip_element_content = true;
                self.skipping_elements_count += 1;
            }
            return;
        }
        let attributes = self.policy.sanitize_attributes(name, &tag.attrs);
        if attributes.is_empty() && !self.policy.allows_element_without_attributes(name) {
            self.skip_closing_tag = true;
            self.closing_tags_to_skip.push(name.to_owned());
            return;
        }
        if self.skip_element_content {
            return;
        }
        self.write_start_tag(name, &attributes);
    }

    /// Apply the policy to a self-closing tag token.
    fn process_self_closing_tag(&mut self, name: &str, tag: &Tag) {
        if is_unsafe_element(&normalise_element_name(name)) {
            return;
        }
        if !self.policy.allows_element(name) {
            return;
        }
        let attributes = self.policy.sanitize_attributes(name, &tag.attrs);
        if attributes.is_empty() && !self.policy.allows_element_without_attributes(name) {
            return;
        }
        if self.skip_element_content {
            return;
        }
        self.write_start_tag(name, &attributes);
    }

    /// Apply the policy to an end tag token.
    fn process_end_tag(&mut self, name: &str) {
        let normalised = normalise_element_name(name);
        if self.most_recently_started_token == normalised {
            self.most_recently_started_token.clear();
        }
        if is_unsafe_element(&normalised) {
            return;
        }
        if self.skip_closing_tag
            && self
                .closing_tags_to_skip
                .last()
                .is_some_and(|pending| pending == name)
        {
            self.closing_tags_to_skip.pop();
            if self.closing_tags_to_skip.is_empty() {
                self.skip_closing_tag = false;
            }
            return;
        }
        if !self.policy.allows_element(name) {
            // No element-name regex is declared by this policy, so a disallowed end
            // tag can never be an allowed element's closing tag.
            if ELEMENTS_SKIPPING_CONTENT.contains(&name) {
                self.skipping_elements_count -= 1;
                if self.skipping_elements_count == 0 {
                    self.skip_element_content = false;
                }
            }
            return;
        }
        if self.skip_element_content {
            return;
        }
        self.write_end_tag(name);
    }
}

/// The token sink that applies the sanitizer state to every token.
struct SanitizeSink<'policy> {
    /// The shared sanitizer state.
    state: Rc<RefCell<SanitizeState<'policy>>>,
}

impl TokenSink for SanitizeSink<'_> {
    type Handle = ();

    fn process_token(&self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        // The tokenizer switches itself into the raw-text state from a
        // fixed tag-name table; html5ever's tokenizer asks its sink to choose. The
        // sink therefore answers with the same transition, before the token is
        // processed, so the element's content is never tokenized as markup.
        let transition = match &token {
            Token::TagToken(tag) if tag.kind == TagKind::StartTag => {
                raw_text_transition(tag.name.as_ref())
            }
            Token::DoctypeToken(_)
            | Token::TagToken(_)
            | Token::CommentToken(_)
            | Token::CharacterTokens(_)
            | Token::NullCharacterToken
            | Token::EOFToken
            | Token::ParseError(_) => None,
        };
        let mut state = self.state.borrow_mut();
        state.process(&token);
        drop(state);
        transition.unwrap_or(TokenSinkResult::Continue)
    }
}

/// The tokenizer state a raw-text start tag selects.
///
/// The HTML tokenizer owns this table; the WHATWG tokenizer html5ever exposes
/// delegates the choice to the token sink. Matching the table is what keeps
/// `<script><img src="/leak"></script>` from emitting the inner image and its text.
fn raw_text_transition(name: &str) -> Option<TokenSinkResult<()>> {
    match name {
        "script" => Some(TokenSinkResult::RawData(RawKind::ScriptData)),
        "style" | "iframe" | "noembed" | "noframes" | "noscript" | "xmp" => {
            Some(TokenSinkResult::RawData(RawKind::Rawtext))
        }
        "textarea" | "title" => Some(TokenSinkResult::RawData(RawKind::Rcdata)),
        "plaintext" => Some(TokenSinkResult::Plaintext),
        _ => None,
    }
}

/// Sanitize author-supplied HTML with the content policy.
///
/// Empty input and whitespace-only input both return unchanged, so blank input
/// short-circuits before tokenizing.
pub(crate) fn sanitize_html(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }
    if html.trim().is_empty() {
        return html.to_owned();
    }
    let policy = &*CONTENT_SANITIZE_POLICY;
    let state = Rc::new(RefCell::new(SanitizeState::new(policy)));
    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let tokenizer = Tokenizer::new(
        SanitizeSink {
            state: Rc::clone(&state),
        },
        TokenizerOpts::default(),
    );
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    state.borrow_mut().take_output()
}

/// Build the content policy plus its four additions.
fn build_content_sanitize_policy() -> SanitizePolicy {
    let mut policy = SanitizePolicy::new();

    // AllowStandardAttributes: language, direction, identity and title.
    policy.allow_global_attributes(&["dir"], pattern(DIRECTION));
    policy.allow_global_attributes(&["lang"], pattern(LANGUAGE));
    policy.allow_global_attributes(&["id"], pattern(IDENTIFIER));
    policy.allow_global_attributes(&["title"], pattern(PARAGRAPH));

    // AllowStandardURLs: parseable URLs that are relative or mailto/http/https,
    // with rel="nofollow" on link elements.
    policy.require_parseable_urls = true;
    policy.allow_relative_urls = true;
    policy.allowed_url_schemes = &["mailto", "http", "https"];
    policy.require_nofollow_on_links = true;

    // Sectioning roots, headings and content grouping.
    policy.allow_elements(&["article", "aside", "figure", "section", "summary"]);
    policy.allow_elements(&["h1", "h2", "h3", "h4", "h5", "h6", "hgroup"]);
    policy.allow_elements(&["br", "div", "hr", "p", "span", "wbr"]);
    policy.allow_element_attributes(&["details"], &["open"], pattern(DETAILS_OPEN));
    policy.allow_element_attributes(&["blockquote"], &["cite"], AttributeValueRule::Any);

    // Links and image maps.
    policy.allow_element_attributes(&["a"], &["href"], AttributeValueRule::Any);
    policy.allow_element_attributes(&["map"], &["name"], pattern(MAP_NAME));
    policy.allow_element_attributes(&["area"], &["alt"], pattern(PARAGRAPH));
    policy.allow_element_attributes(&["area"], &["coords"], pattern(AREA_COORDS));
    policy.allow_element_attributes(&["area"], &["href"], AttributeValueRule::Any);
    policy.allow_element_attributes(&["area"], &["rel"], pattern(SPACE_SEPARATED_TOKENS));
    policy.allow_element_attributes(&["area"], &["shape"], pattern(AREA_SHAPE));
    policy.allow_element_attributes(&["img"], &["usemap"], pattern(USEMAP));

    // Phrase and style elements.
    policy.allow_elements(&[
        "abbr",
        "acronym",
        "cite",
        "code",
        "dfn",
        "em",
        "figcaption",
        "mark",
        "s",
        "samp",
        "strong",
        "sub",
        "sup",
        "var",
    ]);
    policy.allow_element_attributes(&["q"], &["cite"], AttributeValueRule::Any);
    policy.allow_element_attributes(&["time"], &["datetime"], pattern(ISO8601));
    policy.allow_elements(&["b", "i", "pre", "small", "strike", "tt", "u"]);

    // Bidirectional and ruby annotation elements.
    policy.allow_element_attributes(&["bdi", "bdo"], &["dir"], pattern(DIRECTION));
    policy.allow_elements(&["rp", "rt", "ruby"]);

    // Change tracking.
    policy.allow_element_attributes(&["del", "ins"], &["cite"], pattern(PARAGRAPH));
    policy.allow_element_attributes(&["del", "ins"], &["datetime"], pattern(ISO8601));

    // AllowLists.
    policy.allow_element_attributes(&["ol", "ul"], &["type"], pattern(LIST_TYPE));
    policy.allow_element_attributes(&["li"], &["type"], pattern(LIST_TYPE));
    policy.allow_element_attributes(&["li"], &["value"], pattern(INTEGER));
    policy.allow_elements(&["dl", "dt", "dd"]);

    // AllowTables.
    policy.allow_element_attributes(&["table"], &["height", "width"], pattern(NUMBER_OR_PERCENT));
    policy.allow_element_attributes(&["table"], &["summary"], pattern(PARAGRAPH));
    policy.allow_elements(&["caption"]);
    policy.allow_element_attributes(&["col", "colgroup"], &["align"], pattern(CELL_ALIGN));
    policy.allow_element_attributes(
        &["col", "colgroup"],
        &["height", "width"],
        pattern(NUMBER_OR_PERCENT),
    );
    policy.allow_element_attributes(&["col", "colgroup"], &["span"], pattern(INTEGER));
    policy.allow_element_attributes(
        &["col", "colgroup"],
        &["valign"],
        pattern(CELL_VERTICAL_ALIGN),
    );
    policy.allow_element_attributes(&["thead", "tr"], &["align"], pattern(CELL_ALIGN));
    policy.allow_element_attributes(&["thead", "tr"], &["valign"], pattern(CELL_VERTICAL_ALIGN));
    policy.allow_element_attributes(&["td", "th"], &["abbr"], pattern(PARAGRAPH));
    policy.allow_element_attributes(&["td", "th"], &["align"], pattern(CELL_ALIGN));
    policy.allow_element_attributes(&["td", "th"], &["colspan", "rowspan"], pattern(INTEGER));
    policy.allow_element_attributes(&["td", "th"], &["headers"], pattern(SPACE_SEPARATED_TOKENS));
    policy.allow_element_attributes(
        &["td", "th"],
        &["height", "width"],
        pattern(NUMBER_OR_PERCENT),
    );
    policy.allow_element_attributes(&["td", "th"], &["scope"], pattern(SCOPE));
    policy.allow_element_attributes(&["td", "th"], &["valign"], pattern(CELL_VERTICAL_ALIGN));
    // The `(?i)|nowrap` rule has an empty alternative that matches every value; the
    // effective behavior is "any value", which is preserved.
    policy.allow_element_attributes(&["td", "th"], &["nowrap"], AttributeValueRule::Any);
    policy.allow_element_attributes(&["tbody", "tfoot"], &["align"], pattern(CELL_ALIGN));
    policy.allow_element_attributes(
        &["tbody", "tfoot"],
        &["valign"],
        pattern(CELL_VERTICAL_ALIGN),
    );

    // Presentation data: meter and progress values.
    policy.allow_element_attributes(
        &["meter"],
        &["value", "min", "max", "low", "high", "optimum"],
        pattern(NUMBER),
    );
    policy.allow_element_attributes(&["progress"], &["value", "max"], pattern(NUMBER));

    // AllowImages.
    policy.allow_element_attributes(&["img"], &["align"], pattern(IMAGE_ALIGN));
    policy.allow_element_attributes(&["img"], &["alt"], pattern(PARAGRAPH));
    policy.allow_element_attributes(&["img"], &["height", "width"], pattern(NUMBER_OR_PERCENT));
    policy.allow_element_attributes(&["img"], &["src"], AttributeValueRule::Any);

    // The content owner's own additions.
    policy.allow_global_attributes(&["class"], pattern(PARAGRAPH));
    policy.allow_element_attributes(
        &["h1", "h2", "h3", "h4", "h5", "h6"],
        &["id"],
        pattern(HTML_ID),
    );
    policy.allow_element_attributes(
        &["span", "code", "pre"],
        &["style"],
        AttributeValueRule::Any,
    );
    policy.allow_data_attributes = true;

    policy
}

/// Compile one policy pattern, or deny the rule when the literal cannot compile.
///
/// Every pattern below is a literal policy pattern, so the denial branch is
/// unreachable in practice; it exists so a policy typo can never panic or admit a
/// value.
fn pattern(source: &str) -> AttributeValueRule {
    match Regex::new(source) {
        Ok(regex) => AttributeValueRule::Pattern(regex),
        Err(_) => AttributeValueRule::Deny,
    }
}

// The policy's value patterns. `\s` is the ASCII set `[\t\n\f\r ]`, so it is
// spelled out instead of Rust's Unicode `\s`.

/// `(?i)^(rtl|ltr)$`
const DIRECTION: &str = r"(?i)^(rtl|ltr)$";
/// `[a-zA-Z]{2,20}` — unanchored.
const LANGUAGE: &str = r"[a-zA-Z]{2,20}";
/// `[a-zA-Z0-9\:\-_\.]+` — unanchored.
const IDENTIFIER: &str = r"[a-zA-Z0-9:._-]+";
/// `^[A-Za-z][A-Za-z0-9_\-:.]*$` — the content owner's heading id rule.
const HTML_ID: &str = r"^[A-Za-z][A-Za-z0-9:._-]*$";
/// Paragraph-safe value, with ASCII `\s`.
const PARAGRAPH: &str = r"^[\p{L}\p{N}\t\n\x0c\r _',\[\]!./\\()-]*$";
/// Space-separated tokens, with ASCII `\s`.
const SPACE_SEPARATED_TOKENS: &str = r"^([\t\n\x0c\r \p{L}\p{N}_-]+)$";
/// `Integer`
const INTEGER: &str = r"^[0-9]+$";
/// `Number`
const NUMBER: &str = r"^[-+]?[0-9]*\.?[0-9]+([eE][-+]?[0-9]+)?$";
/// `NumberOrPercent`
const NUMBER_OR_PERCENT: &str = r"^[0-9]+[%]?$";
/// `ISO8601` (`time`, `del` and `ins` `datetime`).
///
/// The `.` before the fractional digits is unescaped in the policy pattern, so it
/// matches any character: `1997-07-16T19:20:30x45` survives and is kept here rather
/// than silently narrowed.
const ISO8601: &str = r"^[0-9]{4}(-[0-9]{2}(-[0-9]{2}([ T][0-9]{2}(:[0-9]{2}){1,2}(.[0-9]{1,6})?Z?([-+][0-9]{2}:[0-9]{2})?)?)?)?$";
/// `ListType`
const LIST_TYPE: &str = r"(?i)^(circle|disc|square|a|A|i|I|1)$";
/// `CellAlign`
const CELL_ALIGN: &str = r"(?i)^(center|justify|left|right|char)$";
/// `CellVerticalAlign`
const CELL_VERTICAL_ALIGN: &str = r"(?i)^(baseline|bottom|middle|top)$";
/// `ImageAlign`
const IMAGE_ALIGN: &str =
    r"(?i)^(left|right|top|texttop|middle|absmiddle|baseline|bottom|absbottom)$";
/// The image-map `name` rule.
const MAP_NAME: &str = r"^([\p{L}\p{N}_-]+)$";
/// The image-map `coords` rule.
const AREA_COORDS: &str = r"^([0-9]+,)+[0-9]+$";
/// The image-map `shape` rule.
const AREA_SHAPE: &str = r"(?i)^(default|circle|rect|poly)$";
/// The image `usemap` rule.
const USEMAP: &str = r"(?i)^#[\p{L}\p{N}_-]+$";
/// The table-cell `scope` rule; unanchored.
const SCOPE: &str = r"(?i)(?:row|col)(?:group)?";
/// `(?i)^(|open)$` — the `details` `open` rule.
const DETAILS_OPEN: &str = r"(?i)^(?:open)?$";

/// Whether this normalized element name is one the policy refuses outright.
fn is_unsafe_element(normalised: &str) -> bool {
    normalised == "script" || normalised == "style"
}

/// The elements valid without any attribute.
///
/// It decides whether an allowed element survives after its attributes are filtered
/// out; it is not an allow list.
const ELEMENTS_WITHOUT_ATTRIBUTES: &[&str] = &[
    "abbr",
    "acronym",
    "address",
    "article",
    "aside",
    "audio",
    "b",
    "bdi",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "ins",
    "kbd",
    "li",
    "mark",
    "marquee",
    "nav",
    "ol",
    "optgroup",
    "option",
    "p",
    "picture",
    "pre",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "section",
    "select",
    "small",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "svg",
    "table",
    "tbody",
    "td",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "time",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

/// The elements marked as skip-content.
///
/// When a disallowed element in this set is opened, its character content is not
/// emitted until its closing tag.
const ELEMENTS_SKIPPING_CONTENT: &[&str] = &[
    "frame", "frameset", "iframe", "noembed", "noframes", "noscript", "nostyle", "object",
    "script", "style", "title",
];

/// Whether this element is a link element for the `rel="nofollow"` rule.
fn is_link_element(element: &str) -> bool {
    matches!(element, "a" | "area" | "base" | "link")
}

/// Whether this element/attribute pair carries a URL the policy validates.
fn is_url_attribute(element: &str, attribute: &str) -> bool {
    match element {
        "a" | "area" | "base" | "link" => attribute == "href",
        "blockquote" | "del" | "ins" | "q" => attribute == "cite",
        "audio" | "embed" | "iframe" | "img" | "script" | "source" | "track" | "video" => {
            attribute == "src"
        }
        _ => false,
    }
}

/// Normalize an element name: quote to ASCII and lowercase, escaping non-ASCII.
///
/// A lookalike such as a Cyrillic `і` in `scrіpt` becomes `scr\u0456pt` and can never
/// equal `script`.
fn normalise_element_name(value: &str) -> String {
    let mut normalised = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii() {
            if character == '"' || character == '\\' {
                normalised.push('\\');
            }
            normalised.push(character.to_ascii_lowercase());
            continue;
        }
        let code = u32::from(character);
        if code <= 0xffff {
            let _ = write!(normalised, "\\u{code:04x}");
        } else {
            let _ = write!(normalised, "\\U{code:08x}");
        }
    }
    normalised
}

/// The attribute name the policy sees.
///
/// A single lowercased attribute name is kept, so a prefixed name is rendered as
/// `prefix:local` here.
fn attribute_key(attribute: &html5ever::Attribute) -> String {
    let local: &str = attribute.name.local.as_ref();
    match attribute.name.prefix.as_ref() {
        Some(prefix) => {
            let prefix: &str = prefix.as_ref();
            let mut key = String::with_capacity(prefix.len() + 1 + local.len());
            key.push_str(prefix);
            key.push(':');
            key.push_str(local);
            key
        }
        None => local.to_owned(),
    }
}

/// The data-attribute rule.
fn is_data_attribute(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("data-") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    // `^xml.+` is rejected, so exactly `data-xml` is accepted and only a longer
    // `data-xml...` name is dropped.
    if rest.starts_with("xml") && rest.len() > 3 {
        return false;
    }
    !rest
        .chars()
        .any(|character| character.is_ascii_uppercase() || character == ';')
}

/// Whether the value contains an ASCII control character.
fn has_control_characters(value: &str) -> bool {
    value.chars().any(|character| character.is_ascii_control())
}

/// Whether the value contains a `%` that is not followed by two hex digits.
fn has_invalid_percent_escape(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while let Some(byte) = bytes.get(index) {
        if *byte == b'%' {
            let high = bytes.get(index + 1).copied();
            let low = bytes.get(index + 2).copied();
            if !high.is_some_and(|digit| digit.is_ascii_hexdigit())
                || !low.is_some_and(|digit| digit.is_ascii_hexdigit())
            {
                return true;
            }
            index += 3;
            continue;
        }
        index += 1;
    }
    false
}

/// Classify a value's scheme, distinguishing an absolute location from a relative
/// one.
fn parse_scheme(value: &str) -> Option<Option<String>> {
    let first_segment_end = value
        .char_indices()
        .find_map(|(index, character)| {
            if character == '/' || character == '?' || character == '#' {
                Some(index)
            } else {
                None
            }
        })
        .unwrap_or(value.len());
    let first_segment = value.get(..first_segment_end)?;
    match first_segment.split_once(':') {
        Some((prefix, _rest)) => {
            if is_valid_scheme(prefix) {
                Some(Some(prefix.to_ascii_lowercase()))
            } else {
                None
            }
        }
        None => Some(None),
    }
}

/// Whether the prefix is a valid URL scheme: a letter followed by letters, digits,
/// `+`, `-` or `.`.
fn is_valid_scheme(prefix: &str) -> bool {
    let mut characters = prefix.chars();
    match characters.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    characters
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.'))
}

/// Serialize a syntactically admitted URL with its original escaped component hints.
pub(crate) fn encode_reference_url(raw: &str) -> Option<String> {
    let scheme = parse_scheme(raw)?;
    encode_content_url(raw, scheme.as_deref())
}

/// Encode the URL components consumed by the sanitation policy. Raw query and
/// opaque data remain untouched; paths/fragments retain valid original escapes.
fn encode_content_url(raw: &str, scheme: Option<&str>) -> Option<String> {
    let (before_fragment, fragment) = raw
        .split_once('#')
        .map_or((raw, None), |(a, b)| (a, Some(b)));
    let (address, query) = before_fragment
        .split_once('?')
        .map_or((before_fragment, None), |(a, b)| (a, Some(b)));
    let mut result = String::new();
    let rest = if let Some(scheme) = scheme {
        result.push_str(scheme);
        result.push(':');
        address.get(scheme.len() + 1..)?
    } else {
        address
    };
    if scheme.is_some() && !rest.starts_with('/') {
        result.push_str(rest);
    } else {
        let has_authority =
            rest.starts_with("//") && (scheme.is_some() || !rest.starts_with("///"));
        let (authority, path) = if has_authority {
            let tail = &rest[2..];
            let end = tail.find('/').unwrap_or(tail.len());
            (Some(&tail[..end]), &tail[end..])
        } else {
            (None, rest)
        };
        if let Some(authority) = authority {
            let authority = encode_content_authority(authority, scheme)?;
            if !authority.is_empty() || (scheme.is_some() && !path.is_empty()) {
                result.push_str("//");
                result.push_str(&authority);
            }
        }
        result.push_str(&encode_content_component(path, false)?);
    }
    if let Some(query) = query {
        result.push('?');
        result.push_str(query);
    }
    if let Some(fragment) = fragment.filter(|v| !v.is_empty()) {
        result.push('#');
        result.push_str(&encode_content_component(fragment, true)?);
    }
    Some(result)
}

fn unescape_content_component(raw: &str) -> Option<Vec<u8>> {
    if has_invalid_percent_escape(raw) {
        return None;
    }
    Some(percent_encoding::percent_decode_str(raw).collect())
}
fn content_path_safe(byte: u8, fragment: bool) -> bool {
    byte.is_ascii_alphanumeric()
        || b"-_.~$&+,/:;=@".contains(&byte)
        || (fragment && b"?!()*".contains(&byte))
}
fn encode_content_component(raw: &str, fragment: bool) -> Option<String> {
    let decoded = unescape_content_component(raw)?;
    if raw
        .bytes()
        .all(|byte| content_path_safe(byte, fragment) || b"!'()*[]%".contains(&byte))
    {
        return Some(raw.to_owned());
    }
    let mut result = String::new();
    for byte in decoded {
        if content_path_safe(byte, fragment) {
            result.push(char::from(byte));
        } else {
            let _ = write!(result, "%{byte:02X}");
        }
    }
    Some(result)
}
fn content_host_safe(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-_.~!$&'()*+,;=:[]<>\"".contains(&byte)
}
fn encode_content_authority(authority: &str, scheme: Option<&str>) -> Option<String> {
    let (user, host) = authority
        .rsplit_once('@')
        .map_or((None, authority), |(u, h)| (Some(u), h));
    let mut result = String::new();
    if let Some(user) = user {
        if !crate::url_text::valid_raw_userinfo(user)
        {
            return None;
        }
        let (name, password) = user
            .split_once(':')
            .map_or((user, None), |(n, p)| (n, Some(p)));
        for (index, value) in std::iter::once(name).chain(password).enumerate() {
            if index != 0 {
                result.push(':');
            }
            for byte in unescape_content_component(value)? {
                if byte.is_ascii_alphanumeric() || b"-_.~$&+,;=".contains(&byte) {
                    result.push(char::from(byte));
                } else {
                    let _ = write!(result, "%{byte:02X}");
                }
            }
        }
        result.push('@');
    }
    let port_valid = |port: &str| {
        port.is_empty()
            || port
                .strip_prefix(':')
                .is_some_and(|p| p.bytes().all(|b| b.is_ascii_digit()))
    };
    if host.rfind('[').is_some_and(|index| index != 0) {
        return None;
    }
    let bracketed = host.starts_with('[');
    if bracketed {
        let end = host.rfind(']')?;
        if !port_valid(&host[end + 1..]) {
            return None;
        }
        let decoded = unescape_content_component(&host[1..end])?;
        let text = std::str::from_utf8(&decoded).ok()?;
        let address = match text.split_once('%') {
            Some((_, "")) => return None,
            Some((address, _)) => address,
            None => text,
        };
        address.parse::<std::net::Ipv6Addr>().ok()?;
    } else if let Some(colon) = host.find(':') {
        let index = if matches!(scheme, Some("http" | "https")) {
            colon
        } else {
            host.rfind(':')?
        };
        if !port_valid(&host[index..]) {
            return None;
        }
    }
    let bytes = host.as_bytes();
    let mut index = 0;
    let zone = if bracketed { host.find("%25") } else { None };
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            let escaped = host.get(index..index + 3)?;
            let decoded = unescape_content_component(escaped)?[0];
            if zone.is_some_and(|start| index >= start) {
                if decoded != b'%' && decoded != b' ' && !content_host_safe(decoded) {
                    return None;
                }
            } else if decoded < 128 && escaped != "%25" {
                return None;
            }
            index += 3;
        } else {
            if byte < 128 && !content_host_safe(byte) {
                return None;
            }
            index += 1;
        }
    }
    for byte in unescape_content_component(host)? {
        if content_host_safe(byte) {
            result.push(char::from(byte));
        } else {
            let _ = write!(result, "%{byte:02X}");
        }
    }
    Some(result)
}

/// Whether this element name is a void element for the serializer's element stack.
///
/// This is the serializer's own child-suppression set: it writes no end tag for
/// these names, so the sink must close the element it just opened to keep the stack
/// balanced.
fn is_void_element(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "basefont"
            | "bgsound"
            | "br"
            | "col"
            | "embed"
            | "frame"
            | "hr"
            | "img"
            | "input"
            | "keygen"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// One element name in the HTML namespace for the serializer.
fn qualified_element_name(name: &str) -> QualName {
    QualName::new(None, ns!(html), LocalName::from(name))
}

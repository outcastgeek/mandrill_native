const std = @import("std");
const j = @import("../json/value.zig");
const Value = std.json.Value;
pub const Node = *const Value;
pub const default_base = "https://json-schema.org";
pub const UrlInput = union(enum) { reference: []const u8, anchor: []const u8 };
pub const Url = struct {
    href: []const u8,
    pathname: []const u8,
    hash: []const u8,
    canonicalHref: []const u8,
    /// Classification of the encoded URL hash, including an explicit empty #.
    fragment: enum { absent, empty, pointer, anchor },
    /// These describe the input reference, not the resolved absolute URL.
    fragmentOnly: bool,
    hasScheme: bool,
};
pub const DecodedFragment = struct { kind: enum { pointer, anchor }, text: []const u8 };
pub const Codecs = struct {
    /// WHATWG URL resolution and projections. `anchor` constructs an anchor
    /// URL through the codec. Returned strings live on the supplied allocator.
    url: ?*const fn (?*anyopaque, std.mem.Allocator, UrlInput, []const u8) anyerror!Url = null,
    /// decodeURIComponent(hash without #), with decoded pointer classification.
    fragment: ?*const fn (?*anyopaque, std.mem.Allocator, Url) anyerror!DecodedFragment = null,
    /// Optional replacement for native pointer lookup. Returns a pointer INTO
    /// root, preserving schema identity; null selects the built-in lookup.
    pointer: ?*const fn (?*anyopaque, std.mem.Allocator, Node, []const u8) anyerror!?Node = null,
};
pub const Options = struct {
    context: ?*anyopaque = null,
    documents: Value = .null,
    codecs: Codecs = .{},
    /// Existing host reference capability is an explicit loaded-document
    /// lookup. A null Value means absent. This module never retrieves a URL.
    lookup: ?*const fn (?*anyopaque, []const u8) anyerror!Value = null,
};
fn renderUri(a: std.mem.Allocator, uri: std.Uri) ![]const u8 {
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    try uri.format(&output.writer);
    return output.toOwnedSlice();
}
fn fragmentCharacter(byte: u8) bool {
    return byte >= 0x21 and byte <= 0x7e and switch (byte) {
        '"', '<', '>', '`' => false,
        else => true,
    };
}
fn encodedFragment(a: std.mem.Allocator, fragment: std.Uri.Component) !std.Uri.Component {
    const text = switch (fragment) {
        .raw, .percent_encoded => |value| value,
    };
    var output = std.Io.Writer.Allocating.init(a);
    defer output.deinit();
    // URL fragments retain existing percent triplets; the public component
    // encoder supplies escaping for spaces, Unicode and delimiter characters.
    try std.Uri.Component.percentEncode(&output.writer, text, fragmentCharacter);
    return .{ .percent_encoded = try output.toOwnedSlice() };
}
/// Standard resolution composes the shared HTTP component normalizer for
/// HTTP(S) resources. Full WHATWG relative preprocessing/IDNA remains open.
fn standardUrl(a: std.mem.Allocator, input: UrlInput, base: []const u8) !Url {
    const http_uri = @import("../transport/uri.zig");
    const origin = if (std.ascii.startsWithIgnoreCase(base, "http:") or std.ascii.startsWithIgnoreCase(base, "https:")) try http_uri.parseHttpUrl(a, base) else try std.Uri.parse(base);
    var fragment_only = false;
    var has_scheme = false;
    var uri = switch (input) {
        .anchor => |name| value: {
            var result = origin;
            result.fragment = .{ .raw = name };
            fragment_only = true;
            break :value result;
        },
        .reference => |text| value: {
            fragment_only = std.mem.startsWith(u8, text, "#");
            has_scheme = if (std.Uri.parse(text)) |_| true else |_| false;
            if (std.ascii.eqlIgnoreCase(origin.scheme, "http") or std.ascii.eqlIgnoreCase(origin.scheme, "https")) break :value try http_uri.resolveHttpUrl(a, origin, text);
            var base_path = std.Io.Writer.Allocating.init(a);
            defer base_path.deinit();
            try origin.path.formatPath(&base_path.writer);
            const copied = try std.math.mul(usize, text.len, 2);
            const capacity = try std.math.add(usize, copied, try std.math.add(usize, base_path.written().len, 2));
            const memory = try a.alloc(u8, capacity);
            var remaining = memory;
            @memcpy(memory[0..text.len], text);
            // Returned URI components borrow memory owned by Stack.scratch.
            break :value try std.Uri.resolveInPlace(origin, text.len, &remaining);
        },
    };
    if (std.ascii.eqlIgnoreCase(uri.scheme, "http") or std.ascii.eqlIgnoreCase(uri.scheme, "https")) uri = try http_uri.parseHttpUrl(a, try renderUri(a, uri));
    uri.scheme = try std.ascii.allocLowerString(a, uri.scheme);
    if (uri.host) |host| {
        uri.host = switch (host) {
            .raw => |text| .{ .raw = try std.ascii.allocLowerString(a, text) },
            .percent_encoded => |text| .{ .percent_encoded = try std.ascii.allocLowerString(a, text) },
        };
    }
    if (uri.port) |port| {
        const default_port: ?u16 = if (std.mem.eql(u8, uri.scheme, "http") or std.mem.eql(u8, uri.scheme, "ws")) 80 else if (std.mem.eql(u8, uri.scheme, "https") or std.mem.eql(u8, uri.scheme, "wss")) 443 else if (std.mem.eql(u8, uri.scheme, "ftp")) 21 else null;
        if (default_port == port) uri.port = null;
    }
    if (uri.fragment) |fragment| uri.fragment = try encodedFragment(a, fragment);
    const href = try renderUri(a, uri);
    // Reparse standard serialization so raw anchor input becomes an encoded
    // component before classification and later percent decoding.
    const serialized = try std.Uri.parse(href);
    var canonical = serialized;
    canonical.fragment = null;
    var pathname = std.Io.Writer.Allocating.init(a);
    defer pathname.deinit();
    const path: std.Uri.Component = if (serialized.path.isEmpty() and serialized.host != null) .{ .percent_encoded = "/" } else serialized.path;
    try path.formatPath(&pathname.writer);
    const hash = if (serialized.fragment) |fragment| (if (fragment.isEmpty()) "" else try std.fmt.allocPrint(a, "#{f}", .{std.fmt.alt(fragment, .formatFragment)})) else "";
    const classification: @FieldType(Url, "fragment") = if (serialized.fragment) |fragment|
        (if (fragment.isEmpty()) .empty else if (std.mem.startsWith(u8, switch (fragment) {
            .raw, .percent_encoded => |text| text,
        }, "/")) .pointer else .anchor)
    else
        .absent;
    return .{ .href = href, .pathname = try pathname.toOwnedSlice(), .hash = hash, .canonicalHref = try renderUri(a, canonical), .fragment = classification, .fragmentOnly = fragment_only, .hasScheme = has_scheme };
}
fn standardFragment(a: std.mem.Allocator, url: Url) !DecodedFragment {
    const uri = try std.Uri.parse(url.href);
    const fragment = uri.fragment orelse return error.InvalidSchemaReferenceFragment;
    const encoded = switch (fragment) {
        .raw, .percent_encoded => |value| value,
    };
    if (try @import("../regex/regexp.zig").matches(a, null,
        \\%(?![0-9a-fA-F]{2})
    , encoded, .{})) return error.InvalidSchemaReferenceFragment;
    // Component performs decoding; validation preserves decodeURIComponent's
    // rejection of malformed triplets and invalid UTF-8 without a local lexer.
    const text = try fragment.toRawMaybeAlloc(a);
    if (!std.unicode.utf8ValidateSlice(text)) return error.InvalidSchemaReferenceFragment;
    return .{ .kind = if (std.mem.startsWith(u8, text, "/")) .pointer else .anchor, .text = text };
}
/// Decode one reference token. A true result means another token follows,
/// including the empty final token after a trailing slash.
fn pointerToken(reader: *std.Io.Reader, writer: *std.Io.Writer) !bool {
    while (true) {
        const byte = reader.takeByte() catch |err| switch (err) {
            error.EndOfStream => return false,
            else => return err,
        };
        if (byte == '/') return true;
        if (byte == '~') {
            const next = reader.peekByte() catch |err| switch (err) {
                error.EndOfStream => {
                    try writer.writeByte('~');
                    return false;
                },
                else => return err,
            };
            if (next == '0' or next == '1') {
                reader.toss(1);
                try writer.writeByte(if (next == '1') '/' else '~');
                continue;
            }
        }
        try writer.writeByte(byte);
    }
}
fn pointerChild(node: Node, token: []const u8) ?Node {
    // Pinned Pointer.Get excludes these keys even when they are own members.
    for ([_][]const u8{ "__proto__", "constructor", "prototype" }) |unsafe| if (std.mem.eql(u8, token, unsafe)) return null;
    return switch (node.*) {
        .object => node.object.getPtr(token),
        .array => |array| value: {
            if (token.len == 0 or (token.len > 1 and token[0] == '0')) break :value null;
            for (token) |byte| if (byte < '0' or byte > '9') break :value null;
            const index = std.fmt.parseInt(usize, token, 10) catch break :value null;
            break :value if (index < array.items.len) &array.items[index] else null;
        },
        else => null,
    };
}
fn standardPointer(a: std.mem.Allocator, root: Node, text: []const u8) !?Node {
    if (text.len == 0) return root;
    var reader = std.Io.Reader.fixed(text);
    if (try reader.takeByte() != '/') return null;
    var current = root;
    while (true) {
        var token = std.Io.Writer.Allocating.init(a);
        defer token.deinit();
        // Consume escapes once: ~01 becomes ~1 and is never decoded again.
        const more = try pointerToken(&reader, &token.writer);
        current = pointerChild(current, token.written()) orelse return null;
        if (!more) return current;
    }
}
fn field(node: Node, key: []const u8) ?[]const u8 {
    const value = child(node, key) orelse return null;
    return if (value.* == .string) value.string else null;
}
pub fn child(node: Node, key: []const u8) ?Node {
    return if (node.* == .object) node.object.getPtr(key) else null;
}
fn recursiveAnchor(node: Node) bool {
    const value = child(node, "$recursiveAnchor") orelse return false;
    return value.* == .bool and value.bool;
}
fn same(left: Node, right: Node) bool {
    if (left == right) return true;
    // Guard.IsEqual is identity for containers and strict equality for leaves.
    if (j.number(left.*)) |l| if (j.number(right.*)) |r| return l == r;
    if (std.meta.activeTag(left.*) != std.meta.activeTag(right.*)) return false;
    return switch (left.*) {
        .null => true,
        .bool => |v| v == right.bool,
        .string => |v| std.mem.eql(u8, v, right.string),
        .integer => |v| v == right.integer,
        .float => |v| v == right.float,
        else => false,
    };
}
const Retrieved = struct { base: []const u8, root: Node };
const Frame = struct { schema: Node, base: []const u8, idDepth: usize, resourceDepth: usize };
const Scope = struct {
    lexicalSchema: Node,
    lexicalBase: []const u8,
    referenceBase: []const u8,
    resourceBase: []const u8,
    inRetrievedFrame: bool,
};
pub const Stack = struct {
    root: Node,
    options: Options,
    scratch: std.heap.ArenaAllocator,
    ids: std.ArrayList(Node) = .empty,
    resourceIds: std.ArrayList(Node) = .empty,
    anchors: std.ArrayList(Node) = .empty,
    recursiveAnchors: std.ArrayList(Node) = .empty,
    dynamicAnchors: std.ArrayList(Node) = .empty,
    retrievedResources: std.AutoHashMapUnmanaged(Node, Retrieved) = .{},
    retrievedFrames: std.ArrayList(Frame) = .empty,
    resolvedResources: std.AutoHashMapUnmanaged(Node, Node) = .{},
    loaded: std.StringHashMapUnmanaged(Node) = .{},
    pendingResource: bool = true,
    pub fn init(a: std.mem.Allocator, root: Node, options: Options) @This() {
        return .{ .root = root, .options = options, .scratch = std.heap.ArenaAllocator.init(a) };
    }
    pub fn deinit(self: *@This()) void {
        self.scratch.deinit();
        self.* = undefined;
    }
    fn allocator(self: *@This()) std.mem.Allocator {
        return self.scratch.allocator();
    }
    /// Guard.Keys uses ECMAScript own-property order. Sort borrowed entries,
    /// keeping the original tree and every node identity intact.
    fn entries(self: *@This(), node: Node) ![]j.ObjectEntry {
        return j.objectEntries(self.allocator(), node.object);
    }
    fn url(self: *@This(), input: UrlInput, base: []const u8) !Url {
        if (self.options.codecs.url) |codec| return codec(self.options.context, self.allocator(), input, base);
        return standardUrl(self.allocator(), input, base);
    }
    fn decode(self: *@This(), url_value: Url) !DecodedFragment {
        if (self.options.codecs.fragment) |codec| return codec(self.options.context, self.allocator(), url_value);
        return standardFragment(self.allocator(), url_value);
    }
    fn document(self: *@This(), key: []const u8) !?Node {
        if (self.options.documents == .object) if (self.options.documents.object.getPtr(key)) |node| return node;
        if (self.loaded.get(key)) |node| return node;
        const lookup = self.options.lookup orelse return null;
        const value = try lookup(self.options.context, key);
        if (value == .null) return null;
        const node = try self.allocator().create(Value);
        node.* = value;
        try self.loaded.put(self.allocator(), try self.allocator().dupe(u8, key), node);
        return node;
    }
    fn lastFrame(self: *@This()) ?Frame {
        return if (self.retrievedFrames.items.len == 0) null else self.retrievedFrames.items[self.retrievedFrames.items.len - 1];
    }
    fn buildBase(self: *@This(), ids: []const Node, resource: bool) ![]const u8 {
        const frame = self.lastFrame();
        var base = (try self.url(.{ .reference = if (frame) |f| f.base else default_base }, default_base)).href;
        const depth = if (frame) |f| (if (resource) f.resourceDepth else f.idDepth) else 0;
        for (ids[depth..]) |node| base = (try self.url(.{ .reference = field(node, "$id").? }, base)).href;
        return base;
    }
    fn scope(self: *@This()) !Scope {
        const frame = self.lastFrame();
        const lexical = if (frame) |f| (if (self.ids.items.len > f.idDepth) self.ids.items[self.ids.items.len - 1] else f.schema) else if (self.ids.items.len > 0) self.ids.items[self.ids.items.len - 1] else self.root;
        const lexical_base = try self.buildBase(self.ids.items, false);
        const resource_base = try self.buildBase(self.resourceIds.items, true);
        var reference_base = resource_base;
        if (frame == null and self.ids.items.len > 0) {
            const id = field(self.ids.items[self.ids.items.len - 1], "$id").?;
            if (!(try self.url(.{ .reference = id }, lexical_base)).hasScheme) reference_base = lexical_base;
        }
        return .{ .lexicalSchema = lexical, .lexicalBase = lexical_base, .referenceBase = reference_base, .resourceBase = resource_base, .inRetrievedFrame = frame != null };
    }
    fn resourceAnchors(self: *@This(), node: Node, is_root: bool) anyerror!void {
        if (node.* == .array) {
            for (node.array.items) |*item| try self.resourceAnchors(item, false);
            return;
        }
        if (node.* != .object or (!is_root and field(node, "$id") != null)) return;
        if (!is_root and field(node, "$dynamicAnchor") != null) try self.dynamicAnchors.append(self.allocator(), node);
        for (try self.entries(node)) |entry| try self.resourceAnchors(entry.node, false);
    }
    fn unresourceAnchors(self: *@This(), node: Node, is_root: bool) void {
        if (node.* == .array) {
            for (node.array.items) |*item| self.unresourceAnchors(item, false);
            return;
        }
        if (node.* != .object or (!is_root and field(node, "$id") != null)) return;
        if (!is_root and field(node, "$dynamicAnchor") != null) _ = self.dynamicAnchors.pop();
        var it = node.object.iterator();
        while (it.next()) |entry| self.unresourceAnchors(entry.value_ptr, false);
    }
    fn register(self: *@This(), node: Node) !void {
        const id_count = self.ids.items.len;
        const resource_count = self.resourceIds.items.len;
        const anchor_count = self.dynamicAnchors.items.len;
        const pending = self.pendingResource;
        errdefer {
            self.ids.items.len = id_count;
            self.resourceIds.items.len = resource_count;
            self.dynamicAnchors.items.len = anchor_count;
            self.pendingResource = pending;
        }
        try self.ids.append(self.allocator(), node);
        if (self.pendingResource) try self.resourceIds.append(self.allocator(), node);
        self.pendingResource = false;
        try self.resourceAnchors(node, true);
    }
    fn unregister(self: *@This(), node: Node) void {
        _ = self.ids.pop();
        if (self.resourceIds.items.len > 0 and same(self.resourceIds.items[self.resourceIds.items.len - 1], node)) _ = self.resourceIds.pop();
        self.unresourceAnchors(node, true);
    }
    pub fn push(self: *@This(), node: Node) !void {
        if (node.* != .object) return;
        const before = .{ self.ids.items.len, self.resourceIds.items.len, self.anchors.items.len, self.recursiveAnchors.items.len, self.dynamicAnchors.items.len, self.retrievedFrames.items.len };
        const pending = self.pendingResource;
        errdefer {
            self.ids.items.len = before[0];
            self.resourceIds.items.len = before[1];
            self.anchors.items.len = before[2];
            self.recursiveAnchors.items.len = before[3];
            self.dynamicAnchors.items.len = before[4];
            self.retrievedFrames.items.len = before[5];
            self.pendingResource = pending;
        }
        if (field(node, "$id") != null) try self.register(node);
        if (field(node, "$anchor") != null) try self.anchors.append(self.allocator(), node);
        if (recursiveAnchor(node)) try self.recursiveAnchors.append(self.allocator(), node);
        if (field(node, "$dynamicAnchor") != null) try self.dynamicAnchors.append(self.allocator(), node);
        if (self.retrievedResources.get(node)) |retrieved| try self.retrievedFrames.append(self.allocator(), .{ .schema = retrieved.root, .base = retrieved.base, .idDepth = self.ids.items.len, .resourceDepth = self.resourceIds.items.len });
    }
    pub fn pop(self: *@This(), node: Node) void {
        if (node.* != .object) return;
        if (field(node, "$id") != null) self.unregister(node);
        if (field(node, "$anchor") != null) _ = self.anchors.pop();
        if (recursiveAnchor(node)) _ = self.recursiveAnchors.pop();
        if (field(node, "$dynamicAnchor") != null) _ = self.dynamicAnchors.pop();
        if (self.retrievedResources.contains(node)) _ = self.retrievedFrames.pop();
        if (self.resolvedResources.fetchRemove(node)) |entry| self.unregister(entry.value);
    }
    fn matchHash(self: *@This(), node: Node, ref: Url) !?Node {
        if (ref.fragment == .empty) return node;
        if (ref.fragment == .absent) return null;
        const fragment = try self.decode(ref);
        if (fragment.kind != .pointer) return null;
        if (self.options.codecs.pointer) |codec| return codec(self.options.context, self.allocator(), node, fragment.text);
        return standardPointer(self.allocator(), node, fragment.text);
    }
    fn match(self: *@This(), node: Node, base: Url, ref: Url) !?Node {
        if (field(node, "$id")) |id| {
            if (std.mem.eql(u8, id, ref.hash)) return node;
            const absolute = try self.url(.{ .reference = ref.href }, base.href);
            if (std.mem.eql(u8, base.pathname, absolute.pathname)) {
                if (ref.fragment != .absent and ref.fragment != .empty) {
                    if (try self.matchHash(node, ref)) |found| return found;
                    // MatchId's undefined result continues to both anchor
                    // rules on this same node before the final MatchHash.
                } else return node;
            }
        }
        for ([_][]const u8{ "$anchor", "$dynamicAnchor" }) |keyword| if (field(node, keyword)) |name| {
            const anchor = try self.url(.{ .anchor = name }, base.href);
            const absolute = try self.url(.{ .reference = ref.href }, base.href);
            if (std.mem.eql(u8, anchor.href, absolute.href)) return node;
        };
        return self.matchHash(node, ref);
    }
    fn fromValue(self: *@This(), node: Node, base: Url, ref: Url) anyerror!?Node {
        const next = if (field(node, "$id")) |id| try self.url(.{ .reference = id }, base.href) else base;
        if (node.* == .object) if (try self.match(node, next, ref)) |found| return found;
        var result: ?Node = null;
        switch (node.*) {
            .array => |array| for (array.items) |*item| {
                if (try self.fromValue(item, next, ref)) |found| result = found;
            },
            .object => {
                for (try self.entries(node)) |entry| {
                    if (std.mem.eql(u8, entry.key, "const") or std.mem.eql(u8, entry.key, "enum")) continue;
                    if (try self.fromValue(entry.node, next, ref)) |found| result = found;
                }
            },
            else => {},
        }
        return result;
    }
    fn reference(self: *@This(), root: Node, base: []const u8, ref_text: []const u8, apply_id: bool) !?Node {
        var resolved = try self.url(.{ .reference = if (base.len > 0) base else "." }, default_base);
        if (apply_id) if (field(root, "$id")) |id| {
            resolved = try self.url(.{ .reference = id }, resolved.href);
        };
        const ref = try self.url(.{ .reference = ref_text }, resolved.href);
        if (try self.document(ref_text)) |node| {
            if (node.* != .null) return node;
        }
        if (try self.fromValue(root, resolved, ref)) |node| {
            if (node.* != .null) return node;
        }
        if (std.mem.eql(u8, ref.canonicalHref, resolved.canonicalHref)) return null;
        const remote = try self.document(ref.canonicalHref) orelse return null;
        const remote_base = try self.url(.{ .reference = field(remote, "$id") orelse ref.canonicalHref }, ref.canonicalHref);
        if (ref.hash.len == 0) return remote;
        return self.fromValue(remote, remote_base, ref);
    }
    fn findBase(self: *@This(), node: Node, base: Url, target: Node) anyerror!?[]const u8 {
        if (same(node, target)) return base.href;
        const next = if (field(node, "$id")) |id| try self.url(.{ .reference = id }, base.href) else base;
        switch (node.*) {
            .array => |array| for (array.items) |*item| {
                if (try self.findBase(item, next, target)) |found| return found;
            },
            .object => {
                for (try self.entries(node)) |entry| {
                    if (try self.findBase(entry.node, next, target)) |found| return found;
                }
            },
            else => {},
        }
        return null;
    }
    pub fn resolveRef(self: *@This(), text: []const u8) !?Node {
        const frame = try self.scope();
        const url_value = try self.url(.{ .reference = text }, frame.referenceBase);
        const source = if (frame.inRetrievedFrame) frame.lexicalSchema else self.root;
        const ref_root = if (url_value.fragmentOnly) frame.lexicalSchema else source;
        const target = try self.reference(ref_root, frame.referenceBase, text, false) orelse return null;
        self.pendingResource = true;
        if (target.* != .object) return target;
        const remote = !std.mem.eql(u8, url_value.canonicalHref, frame.resourceBase);
        var retrieved: ?Retrieved = null;
        if (remote) if (try self.document(url_value.canonicalHref)) |root| {
            if (root.* == .object) retrieved = .{ .base = url_value.canonicalHref, .root = root };
        };
        if (retrieved == null and url_value.fragmentOnly and child(self.root, "$schema") == null) {
            const base = try self.url(.{ .reference = if (frame.referenceBase.len > 0) frame.referenceBase else "." }, default_base);
            if (try self.findBase(frame.lexicalSchema, base, target)) |target_base| {
                if (!std.mem.eql(u8, target_base, frame.referenceBase)) retrieved = .{ .base = target_base, .root = frame.lexicalSchema };
            }
        }
        var resolved_resource: ?Node = null;
        if (remote and field(target, "$id") == null) {
            if (try self.reference(self.root, frame.referenceBase, url_value.canonicalHref, false)) |resource| {
                var included = false;
                for (self.ids.items) |id| {
                    if (same(id, resource)) {
                        included = true;
                        break;
                    }
                }
                if (resource.* == .object and field(resource, "$id") != null and !included) {
                    resolved_resource = resource;
                }
            }
        }
        // Reserve both maps before entering a resource. Once registration
        // succeeds, applying the resolution cannot fail partway through.
        if (resolved_resource != null) try self.resolvedResources.ensureUnusedCapacity(self.allocator(), 1);
        if (retrieved != null) try self.retrievedResources.ensureUnusedCapacity(self.allocator(), 1);
        if (resolved_resource) |resource| {
            try self.register(resource);
            self.resolvedResources.putAssumeCapacity(target, resource);
        }
        if (retrieved) |value| self.retrievedResources.putAssumeCapacity(target, value);
        return target;
    }
    fn firstDynamic(self: *@This(), name: []const u8) ?Node {
        for (self.dynamicAnchors.items) |node| if (std.mem.eql(u8, field(node, "$dynamicAnchor").?, name)) return node;
        return null;
    }
    fn findDynamic(self: *@This(), node: Node, name: []const u8) anyerror!?Node {
        if (field(node, "$dynamicAnchor")) |anchor| if (std.mem.eql(u8, anchor, name)) return node;
        switch (node.*) {
            .array => |array| for (array.items) |*item| {
                if (try self.findDynamic(item, name)) |found| return found;
            },
            .object => {
                for (try self.entries(node)) |entry| {
                    if (try self.findDynamic(entry.node, name)) |found| return found;
                }
            },
            else => {},
        }
        return null;
    }
    pub fn resolveDynamicRef(self: *@This(), text: []const u8) !?Node {
        const frame = try self.scope();
        const url_value = try self.url(.{ .reference = text }, frame.lexicalBase);
        const root = if (url_value.fragmentOnly) frame.lexicalSchema else self.root;
        const target = try self.reference(root, frame.lexicalBase, text, false);
        const result: ?Node = if (target) |node| blk: {
            const anchor = field(node, "$dynamicAnchor") orelse break :blk node;
            if (url_value.fragment == .pointer) break :blk node;
            break :blk self.firstDynamic(anchor) orelse node;
        } else blk: {
            if (url_value.fragment != .anchor) break :blk null;
            const fragment = try self.decode(url_value);
            break :blk self.firstDynamic(fragment.text) orelse try self.findDynamic(self.root, fragment.text);
        };
        if (result) |node| {
            if (j.truthy(node.*)) self.pendingResource = true;
        }
        return result;
    }
    pub fn resolveRecursiveRef(self: *@This(), text: []const u8) !?Node {
        const frame = try self.scope();
        const root = if (recursiveAnchor(frame.lexicalSchema)) self.recursiveAnchors.items[0] else frame.lexicalSchema;
        const result = try self.reference(root, frame.lexicalBase, text, false);
        if (result) |node| {
            if (j.truthy(node.*)) self.pendingResource = true;
        }
        return result;
    }
};

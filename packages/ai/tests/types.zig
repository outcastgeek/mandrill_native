const std=@import("std");
const ai=@import("ai");
const t=ai.types;
const mock=@import("mocks.zig");
test "all message roles retain tagged wire fields and owned snapshots" {
    var arena=std.heap.ArenaAllocator.init(std.testing.allocator); defer arena.deinit(); const a=arena.allocator();
    const messages=[]const t.Message=&.{
        .{.system=.{.content=.{.string="System"},.timestamp=1,.sections=try ai.ai.json_value.from(a,.{.policy="one"})}},
        .{.user=.{.content=.{.blocks=&.{.{.text=.{.text="hello"}},.{.image=.{.mimeType="image/png",.data="AA=="}}}},.timestamp=2}},
        .{.assistant=.{.api="custom-api",.provider="custom-provider",.model="model",.timestamp=3,.stopReason=.toolUse,.content=&.{.{.thinking=.{.thinking="reason",.thinkingSignature="opaque",.redacted=false}},.{.toolCall=.{.id="call",.name="tool",.arguments=try ai.ai.json_value.from(a,.{.enabled=false,.count=0,.nothing=@as(?u8,null)})}}}}},
        .{.toolResult=.{.toolCallId="call",.toolName="tool",.content=&.{.{.text=.{.text="result"}}},.isError=false,.timestamp=4,.details=try ai.ai.json_value.from(a,.{.value=0})}},
    };
    var copy=try t.Owned([]const t.Message).clone(std.testing.allocator,messages); defer copy.deinit();
    const bytes=try std.json.Stringify.valueAlloc(a,copy.value(),.{}); const decoded=try std.json.parseFromSlice(std.json.Value,a,bytes,.{}); defer decoded.deinit();
    try std.testing.expectEqualStrings("assistant",ai.ai.json_value.field(decoded.value.array.items[2],"role").?);
    try std.testing.expectEqualStrings("opaque",copy.value()[2].assistant.content[0].thinking.thinkingSignature.?);
    try std.testing.expect(!copy.value()[3].toolResult.isError);
    try std.testing.expectEqualStrings("custom-api",copy.value()[2].assistant.api);
}
fn cloneAllocationFailures(a: std.mem.Allocator) !void {
    const model=mock.model("custom-api"); var copy=try t.Owned(t.Model).clone(a,model); defer copy.deinit(); try std.testing.expectEqualStrings(model.id,copy.value().id);
}
test "owned public values clean up every failed allocation" { try std.testing.checkAllAllocationFailures(std.testing.allocator,cloneAllocationFailures,.{}); }
test "OAuth unknown metadata survives standard JSON roundtrip" {
    const source="{\"type\":\"oauth\",\"access\":\"access\",\"refresh\":\"refresh\",\"expires\":42,\"accountId\":\"account\",\"availableModelIds\":[\"one\"]}";
    const parsed=try std.json.parseFromSlice(ai.auth.Credential,std.testing.allocator,source,.{}); defer parsed.deinit();
    const encoded=try std.json.Stringify.valueAlloc(std.testing.allocator,parsed.value,.{}); defer std.testing.allocator.free(encoded);
    const value=try std.json.parseFromSlice(std.json.Value,std.testing.allocator,encoded,.{}); defer value.deinit();
    try std.testing.expectEqualStrings("account",ai.ai.json_value.field(value.value,"accountId").?);
    try std.testing.expect(ai.ai.json_value.get(value.value,"metadata") == null);
}

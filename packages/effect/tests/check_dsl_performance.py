"""Reproducible DSL runtime/allocation and named compile-size measurements.
Measurements are evidence, with no unsupported relative-speed assertion.
Run serially after correctness checks for controlled final measurements.
"""
from dsl_gate_evidence import fingerprint, completed
_gate_before = fingerprint('performance')

import argparse
import hashlib
import json
import platform
import subprocess
import time
from pathlib import Path

PACKAGE = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('zig')
parser.add_argument('--sizes', default='1,2,4,8,16,32,64')
parser.add_argument('--families', default='stage,binding,branch,field')
args = parser.parse_args()
output = PACKAGE / 'zig-out/dsl-performance'
output.mkdir(parents=True, exist_ok=True)
common = '''const std=@import("std"); const fx=@import("effect");
const R=struct { allocator:std.mem.Allocator, input:u32 };
const P=fx.Effect(u32,void,R); const p=fx.dsl.program;
fn require(condition:bool) void {if(!condition) @panic("performance fixture result mismatch");}
fn seed(init:std.process.Init.Minimal) u32 {var argv=init.args.iterate();_=argv.next();return std.fmt.parseInt(u32,argv.next() orelse @panic("missing seed"),10) catch @panic("invalid seed");}
fn read(env:*const R) u32 {return env.input;}
fn next(value:*const u32,_:*const R) u32 {return value.* +% 1;}
fn mapped(value:u32) u32 {return value +% 1;}
fn predicate(_: *const u32,env:*const R) bool {return env.input % 2 == 0;}
fn yes(value:*const u32,_:*const R) P {return P.succeed(value.* +% 1);}
fn no(value:*const u32,_:*const R) P {return P.succeed(value.* +% 2);}
'''


def fixture(family, count):
    if family == 'stage':
        expression = 'fx.dsl.pipeline.pipe(P.sync(read), .{' + ','.join(['fx.dsl.pipeline.map(mapped)'] * count) + '})'
        check = f'require(result.success==input +% {count});'
    elif family in ('binding', 'branch'):
        nodes = ['p.native("s0", .{}, read)']
        for i in range(1, count + 1):
            nodes.append(f'p.native("s{i}", .{{"s{i-1}"}}, next)' if family == 'binding' else f'p.branch("s{i}", .{{"s{i-1}"}}, predicate, yes, no)')
        expression = 'p.build(void,R,.{' + ','.join(nodes) + '})'
        check = f'require(result.success==input +% @as(u32, {count}) *% (if(input%2==0) @as(u32,1) else 2));' if family == 'branch' else f'require(result.success==input +% {count});'
    else:
        fields = ','.join(f'.f{i}=.{{.codec=fx.JsonCodec.signed_integer,.presence=.required}}' for i in range(count))
        document = json.dumps(json.dumps({f'f{i}': i for i in range(count)}, separators=(',', ':')))
        assertions = ''.join(f'require(parsed.success.value.f{i}=={i});' for i in range(count))
        return common + f'''const Record=fx.dsl.schemaRecord(R,.{{.fields=.{{{fields}}},.policy=.{{.unknown_fields=.reject,.duplicate_fields=.reject,.issues=.first}}}});
pub fn main(init:std.process.Init.Minimal) void {{const env:R=.{{.allocator=std.heap.page_allocator,.input=0}};
var argv=init.args.iterate();_=argv.next();_=argv.next();const document=argv.next() orelse @panic("missing document");var parsed=Record.parseBytesDirect(&env,document,.{{.max_document_bytes=4096,.max_depth=8,.max_value_bytes=128,.max_integer_digits=16}});defer parsed.deinit();require(parsed==.success);{assertions}}}
'''
    return common + f'''const program={expression};
pub fn main(init:std.process.Init.Minimal) void {{const supplied=seed(init); var checksum:u32=0; for([_]u32{{supplied,supplied+%1,supplied+%17,supplied+%255}}) |input| {{const env:R=.{{.allocator=std.heap.page_allocator,.input=input}};var result=fx.run(&env,program);defer result.deinit();require(result==.success);{check}checksum +%= result.success;}}std.debug.print("{{d}}\\n",.{{checksum}});}}
'''


def command(command, timeout=120):
    start = time.perf_counter()
    checked = subprocess.run(command, cwd=PACKAGE, text=True, capture_output=True, timeout=timeout)
    elapsed = time.perf_counter() - start
    if checked.returncode:
        raise SystemExit(' '.join(map(str, command)) + '\n' + checked.stdout + checked.stderr)
    return checked, elapsed


digest = hashlib.sha256()
for path in sorted((PACKAGE / 'src').rglob('*.zig')):
    digest.update(str(path.relative_to(PACKAGE)).encode())
    digest.update(path.read_bytes())
report = dict(compiler=command([args.zig, 'version'])[0].stdout.strip(), platform=platform.platform(), optimize='fast', backend='LLVM', scaling_entrypoint='std.process.Init.Minimal with page allocator; pure fixtures do not initialize an I/O runtime', cache_state='Normal Zig cache; shared dependency artifacts may be warm. Compile seconds are process wall time, not a cold-cache claim.', source_sha256=digest.hexdigest(), scaling=[], runtime=None)
for family in args.families.split(','):
    if family not in ('stage', 'binding', 'branch', 'field'):
        raise SystemExit('unknown family ' + family)
    for count in map(int, args.sizes.split(',')):
        source = output / f'{family}-{count}.zig'
        source.write_text(fixture(family, count))
        binary = output / f'{family}-{count}'
        cmd = [args.zig, 'build-exe', '-fllvm', '-O', 'fast', '-femit-bin=' + str(binary), '--dep', 'effect', '-Mroot=' + str(source), '-O', 'fast', '-Meffect=' + str(PACKAGE / 'src/root.zig')]
        _, compile_seconds = command(cmd)
        run_args=[str(binary), '17']
        if family == 'field': run_args.append(json.dumps({f'f{i}': i for i in range(count)},separators=(',', ':')))
        _, run_seconds = command(run_args)
        entry = dict(family=family, count=count, compile_seconds=compile_seconds, execute_process_seconds=run_seconds, executable_bytes=binary.stat().st_size, command=cmd)
        report['scaling'].append(entry)
        print(f'{family}-{count}: compiled and executed; {compile_seconds:.3f}s, {entry["executable_bytes"]} executable bytes', flush=True)
source = PACKAGE / 'proofs/dsl-performance.zig'
binary = output / 'runtime'
command([args.zig, 'build-exe', '-fllvm', '-O', 'fast', '-femit-bin=' + str(binary), '--dep', 'effect', '-Mroot=' + str(source), '-O', 'fast', '-Meffect=' + str(PACKAGE / 'src/root.zig')])
measured, elapsed = command([str(binary), '17'])
report['runtime'] = json.loads(measured.stderr)
report['runtime_process_seconds'] = elapsed
(output / 'measurements.json').write_text(json.dumps(report, indent=2) + '\n')
print('Measurements saved to', output / 'measurements.json')

completed("performance", "fast", _gate_before, dict(measurements=str(output / "measurements.json")))

use super::RegisterP;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, UserFuncName, types};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::collections::VecDeque;
use std::mem;

const STATE_A_OFFSET: i32 = mem::offset_of!(TraceCpuState, a) as i32;
const STATE_X_OFFSET: i32 = mem::offset_of!(TraceCpuState, x) as i32;
const STATE_Y_OFFSET: i32 = mem::offset_of!(TraceCpuState, y) as i32;
const STATE_SP_OFFSET: i32 = mem::offset_of!(TraceCpuState, sp) as i32;
const STATE_P_OFFSET: i32 = mem::offset_of!(TraceCpuState, p) as i32;
const STATE_PC_OFFSET: i32 = mem::offset_of!(TraceCpuState, pc) as i32;
const STATE_CYCLES_OFFSET: i32 = mem::offset_of!(TraceCpuState, cycles) as i32;

type TraceFn = unsafe extern "C" fn(*mut TraceCpuState);
const MAX_TRACE_BYTES: usize = 32;
const MAX_CACHED_TRACES: usize = 16;
const MAX_TRACE_HOTNESS_ENTRIES: usize = 128;
const COMPILE_HIT_THRESHOLD: u8 = 8;
const MAX_TRACE_EVICTIONS: u8 = 2;

#[repr(C)]
pub(crate) struct TraceCpuState {
    a: u64,
    x: u64,
    y: u64,
    sp: u64,
    p: u64,
    pc: u64,
    cycles: u64,
}

impl TraceCpuState {
    pub(crate) fn new(pc: u16, a: u8, x: u8, y: u8, sp: u8, p: u8) -> Self {
        Self {
            a: u64::from(a),
            x: u64::from(x),
            y: u64::from(y),
            sp: u64::from(sp),
            p: u64::from(p),
            pc: u64::from(pc),
            cycles: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TracePlan {
    entry_pc: u16,
    ops: Vec<TraceOp>,
    exit: TraceExit,
    guard_bytes: Vec<u8>,
    cycle_upper_bound: u64,
}

impl TracePlan {
    pub(crate) fn decode_block<F>(entry_pc: u16, mut read: F) -> Option<Self>
    where
        F: FnMut(u16) -> Option<u8>,
    {
        let mut pc = entry_pc;
        let mut decoded_bytes = 0usize;
        let mut guard_bytes = Vec::new();
        let mut ops = Vec::new();
        let mut op_cycles = 0u64;

        loop {
            if decoded_bytes >= MAX_TRACE_BYTES {
                return None;
            }

            let opcode = read(pc)?;
            guard_bytes.push(opcode);
            decoded_bytes += 1;

            if let Some(condition) = BranchCondition::from_opcode(opcode) {
                if decoded_bytes + 1 > MAX_TRACE_BYTES {
                    return None;
                }
                let offset_byte = read(pc.wrapping_add(1))?;
                guard_bytes.push(offset_byte);

                let fallthrough_pc = pc.wrapping_add(2);
                let taken_pc = fallthrough_pc.wrapping_add_signed(i16::from(offset_byte as i8));
                read(fallthrough_pc)?;
                read(taken_pc)?;
                let taken_cycles = branch_cycles(pc, taken_pc, true);
                let fallthrough_cycles = branch_cycles(pc, fallthrough_pc, false);
                let cycle_upper_bound = op_cycles + u64::from(taken_cycles.max(fallthrough_cycles));

                return Some(Self {
                    entry_pc,
                    ops,
                    exit: TraceExit::Branch {
                        condition,
                        branch_pc: pc,
                        taken_pc,
                        fallthrough_pc,
                        taken_cycles,
                        fallthrough_cycles,
                    },
                    guard_bytes,
                    cycle_upper_bound,
                });
            }

            let op = TraceOp::decode(opcode, pc, &mut read)?;
            if decoded_bytes + usize::from(op.len() - 1) > MAX_TRACE_BYTES {
                return None;
            }
            if let Some(value) = op.immediate_byte() {
                guard_bytes.push(value);
            }
            decoded_bytes += usize::from(op.len() - 1);
            op_cycles += u64::from(op.cycles());
            pc = pc.wrapping_add(u16::from(op.len()));
            ops.push(op);
        }
    }

    fn guard_bytes(&self) -> &[u8] {
        &self.guard_bytes
    }

    fn cycle_upper_bound(&self) -> u64 {
        self.cycle_upper_bound
    }

    fn exits_are_peek_safe<F>(&self, read: &mut F) -> bool
    where
        F: FnMut(u16) -> Option<u8>,
    {
        let &TraceExit::Branch {
            taken_pc,
            fallthrough_pc,
            ..
        } = &self.exit;
        read(fallthrough_pc).is_some() && read(taken_pc).is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraceOp {
    Dex,
    Dey,
    Inx,
    Iny,
    Tax,
    Tay,
    Txa,
    Tya,
    Tsx,
    Txs,
    LdaImm(u8),
    LdxImm(u8),
    LdyImm(u8),
    CmpImm(u8),
    CpxImm(u8),
    CpyImm(u8),
    Clc,
    Sec,
    Cld,
    Sed,
    Clv,
    Nop,
}

impl TraceOp {
    pub(crate) fn is_supported_block_start(opcode: u8) -> bool {
        Self::decode_without_operands(opcode).is_some()
    }

    fn decode<F>(opcode: u8, pc: u16, read: &mut F) -> Option<Self>
    where
        F: FnMut(u16) -> Option<u8>,
    {
        let mut imm = || read(pc.wrapping_add(1));
        match opcode {
            0xCA => Some(Self::Dex),
            0x88 => Some(Self::Dey),
            0xE8 => Some(Self::Inx),
            0xC8 => Some(Self::Iny),
            0xAA => Some(Self::Tax),
            0xA8 => Some(Self::Tay),
            0x8A => Some(Self::Txa),
            0x98 => Some(Self::Tya),
            0xBA => Some(Self::Tsx),
            0x9A => Some(Self::Txs),
            0xA9 => Some(Self::LdaImm(imm()?)),
            0xA2 => Some(Self::LdxImm(imm()?)),
            0xA0 => Some(Self::LdyImm(imm()?)),
            0xC9 => Some(Self::CmpImm(imm()?)),
            0xE0 => Some(Self::CpxImm(imm()?)),
            0xC0 => Some(Self::CpyImm(imm()?)),
            0x18 => Some(Self::Clc),
            0x38 => Some(Self::Sec),
            0xD8 => Some(Self::Cld),
            0xF8 => Some(Self::Sed),
            0xB8 => Some(Self::Clv),
            0xEA => Some(Self::Nop),
            _ => None,
        }
    }

    fn decode_without_operands(opcode: u8) -> Option<Self> {
        match opcode {
            0xCA => Some(Self::Dex),
            0x88 => Some(Self::Dey),
            0xE8 => Some(Self::Inx),
            0xC8 => Some(Self::Iny),
            0xAA => Some(Self::Tax),
            0xA8 => Some(Self::Tay),
            0x8A => Some(Self::Txa),
            0x98 => Some(Self::Tya),
            0xBA => Some(Self::Tsx),
            0x9A => Some(Self::Txs),
            0xA9 => Some(Self::LdaImm(0)),
            0xA2 => Some(Self::LdxImm(0)),
            0xA0 => Some(Self::LdyImm(0)),
            0xC9 => Some(Self::CmpImm(0)),
            0xE0 => Some(Self::CpxImm(0)),
            0xC0 => Some(Self::CpyImm(0)),
            0x18 => Some(Self::Clc),
            0x38 => Some(Self::Sec),
            0xD8 => Some(Self::Cld),
            0xF8 => Some(Self::Sed),
            0xB8 => Some(Self::Clv),
            0xEA => Some(Self::Nop),
            _ => None,
        }
    }

    fn len(self) -> u8 {
        match self {
            Self::LdaImm(_)
            | Self::LdxImm(_)
            | Self::LdyImm(_)
            | Self::CmpImm(_)
            | Self::CpxImm(_)
            | Self::CpyImm(_) => 2,
            _ => 1,
        }
    }

    fn cycles(self) -> u8 {
        2
    }

    fn immediate_byte(self) -> Option<u8> {
        match self {
            Self::LdaImm(value)
            | Self::LdxImm(value)
            | Self::LdyImm(value)
            | Self::CmpImm(value)
            | Self::CpxImm(value)
            | Self::CpyImm(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TraceExit {
    Branch {
        condition: BranchCondition,
        branch_pc: u16,
        taken_pc: u16,
        fallthrough_pc: u16,
        taken_cycles: u8,
        fallthrough_cycles: u8,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BranchCondition {
    Plus,
    Minus,
    OverflowClear,
    OverflowSet,
    CarryClear,
    CarrySet,
    NotZero,
    Equal,
}

impl BranchCondition {
    fn from_opcode(opcode: u8) -> Option<Self> {
        match opcode {
            0x10 => Some(Self::Plus),
            0x30 => Some(Self::Minus),
            0x50 => Some(Self::OverflowClear),
            0x70 => Some(Self::OverflowSet),
            0x90 => Some(Self::CarryClear),
            0xB0 => Some(Self::CarrySet),
            0xD0 => Some(Self::NotZero),
            0xF0 => Some(Self::Equal),
            _ => None,
        }
    }
}

pub(crate) struct CompiledTrace {
    // Keep the JIT module alive for as long as the function pointer can be
    // called. This intentionally avoids a self-referential CPU cache: each
    // cached trace owns the module that owns its finalized code.
    _module: JITModule,
    func: TraceFn,
}

impl CompiledTrace {
    fn run(&self, state: &mut TraceCpuState) {
        // SAFETY: TraceCompiler creates functions with exactly this ABI and the
        // generated IR only touches the supplied TraceCpuState.
        unsafe { (self.func)(state) }
    }
}

pub(crate) struct TraceCompiler;

impl TraceCompiler {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn compile(&mut self, plan: &TracePlan) -> Option<CompiledTrace> {
        Self::compile_block(plan)
    }

    fn compile_block(plan: &TracePlan) -> Option<CompiledTrace> {
        let builder = JITBuilder::new(cranelift_module::default_libcall_names()).ok()?;
        let mut module = JITModule::new(builder);
        let pointer_type = module.target_config().pointer_type();
        let mut ctx = module.make_context();
        ctx.func.signature.call_conv = CallConv::triple_default(module.isa().triple());
        ctx.func.signature.params.push(AbiParam::new(pointer_type));
        ctx.func.name = UserFuncName::user(0, 0);
        let name = "trace_jit_block";
        let func_id = module
            .declare_function(name, Linkage::Local, &ctx.func.signature)
            .ok()?;

        let mut builder_context = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_context);
        let entry = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        builder.switch_to_block(entry);
        let state_ptr = builder.block_params(entry)[0];

        let mut a = load_i64(&mut builder, state_ptr, STATE_A_OFFSET);
        let mut x = load_i64(&mut builder, state_ptr, STATE_X_OFFSET);
        let mut y = load_i64(&mut builder, state_ptr, STATE_Y_OFFSET);
        let mut sp = load_i64(&mut builder, state_ptr, STATE_SP_OFFSET);
        let mut p = load_i64(&mut builder, state_ptr, STATE_P_OFFSET);
        let base_cycles = load_i64(&mut builder, state_ptr, STATE_CYCLES_OFFSET);
        let mut op_cycles = 0i64;

        for op in &plan.ops {
            emit_op(&mut builder, *op, &mut a, &mut x, &mut y, &mut sp, &mut p);
            op_cycles += i64::from(op.cycles());
        }

        let &TraceExit::Branch {
            condition,
            taken_pc,
            fallthrough_pc,
            taken_cycles,
            fallthrough_cycles,
            ..
        } = &plan.exit;
        let branch_taken = emit_branch_condition(&mut builder, condition, p);
        let taken_pc = iconst(&mut builder, i64::from(taken_pc));
        let fallthrough_pc = iconst(&mut builder, i64::from(fallthrough_pc));
        let final_pc = builder.ins().select(branch_taken, taken_pc, fallthrough_pc);
        let taken_cycles = iconst(&mut builder, op_cycles + i64::from(taken_cycles));
        let fallthrough_cycles = iconst(&mut builder, op_cycles + i64::from(fallthrough_cycles));
        let trace_cycles = builder
            .ins()
            .select(branch_taken, taken_cycles, fallthrough_cycles);
        let final_cycles = builder.ins().iadd(base_cycles, trace_cycles);

        let a = mask_u8(&mut builder, a);
        let x = mask_u8(&mut builder, x);
        let y = mask_u8(&mut builder, y);
        let sp = mask_u8(&mut builder, sp);
        let p = mask_u8(&mut builder, p);
        store_i64(&mut builder, state_ptr, STATE_A_OFFSET, a);
        store_i64(&mut builder, state_ptr, STATE_X_OFFSET, x);
        store_i64(&mut builder, state_ptr, STATE_Y_OFFSET, y);
        store_i64(&mut builder, state_ptr, STATE_SP_OFFSET, sp);
        store_i64(&mut builder, state_ptr, STATE_P_OFFSET, p);
        store_i64(&mut builder, state_ptr, STATE_PC_OFFSET, final_pc);
        store_i64(&mut builder, state_ptr, STATE_CYCLES_OFFSET, final_cycles);
        builder.ins().return_(&[]);
        builder.seal_all_blocks();
        builder.finalize();

        module.define_function(func_id, &mut ctx).ok()?;
        module.clear_context(&mut ctx);
        module.finalize_definitions().ok()?;
        let code = module.get_finalized_function(func_id);
        Some(CompiledTrace {
            // SAFETY: The finalized Cranelift function was declared and built
            // with the same ABI as TraceFn.
            func: unsafe { mem::transmute::<*const u8, TraceFn>(code) },
            _module: module,
        })
    }
}

impl Default for TraceCompiler {
    fn default() -> Self {
        Self::new()
    }
}

struct CachedTrace {
    entry_pc: u16,
    guard_bytes: Vec<u8>,
    plan: TracePlan,
    trace: CompiledTrace,
}

struct TraceHotness {
    entry_pc: u16,
    guard_bytes: Vec<u8>,
    hits: u8,
    evictions: u8,
    blocked: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct TraceInput {
    pub(crate) entry_pc: u16,
    pub(crate) a: u8,
    pub(crate) x: u8,
    pub(crate) y: u8,
    pub(crate) sp: u8,
    pub(crate) p: u8,
    pub(crate) max_cycles: u64,
}

pub(crate) struct TraceRun {
    pub(crate) pc_to_fetch: u16,
    pub(crate) a: u8,
    pub(crate) x: u8,
    pub(crate) y: u8,
    pub(crate) sp: u8,
    pub(crate) p: u8,
    pub(crate) cycles: u64,
}

#[derive(Default)]
pub(crate) struct TraceJit {
    cache: VecDeque<CachedTrace>,
    hotness: VecDeque<TraceHotness>,
}

impl TraceJit {
    pub(crate) fn clear(&mut self) {
        self.cache.clear();
        self.hotness.clear();
    }

    pub(crate) fn run_block_trace<F>(
        &mut self,
        input: TraceInput,
        mut read_code: F,
    ) -> Option<TraceRun>
    where
        F: FnMut(u16) -> Option<u8>,
    {
        if let Some(index) = self.cache_index(input.entry_pc) {
            if !self.cached_guard_matches(index, &mut read_code) {
                self.cache.remove(index);
                return None;
            }
            if !self.cache[index].plan.exits_are_peek_safe(&mut read_code) {
                return None;
            }
            return self.run_cached(index, input);
        }

        let plan = TracePlan::decode_block(input.entry_pc, &mut read_code)?;
        if plan.cycle_upper_bound() > input.max_cycles {
            return None;
        }
        if !self.observe_hot_trace(&plan) {
            return None;
        }
        let mut compiler = TraceCompiler::new();
        let trace = compiler.compile(&plan)?;
        self.cache.push_front(CachedTrace {
            entry_pc: input.entry_pc,
            guard_bytes: plan.guard_bytes().to_vec(),
            plan,
            trace,
        });
        while self.cache.len() > MAX_CACHED_TRACES {
            if let Some(evicted) = self.cache.pop_back() {
                self.record_evicted_trace(&evicted);
            }
        }
        self.run_cached(0, input)
    }

    fn cache_index(&self, entry_pc: u16) -> Option<usize> {
        self.cache
            .iter()
            .position(|cached| cached.entry_pc == entry_pc)
    }

    fn cached_guard_matches<F>(&self, index: usize, read_code: &mut F) -> bool
    where
        F: FnMut(u16) -> Option<u8>,
    {
        let cached = &self.cache[index];
        cached
            .guard_bytes
            .iter()
            .enumerate()
            .all(|(offset, expected)| {
                read_code(cached.entry_pc.wrapping_add(offset as u16)) == Some(*expected)
            })
    }

    fn observe_hot_trace(&mut self, plan: &TracePlan) -> bool {
        if let Some(index) = self
            .hotness
            .iter()
            .position(|trace| trace.entry_pc == plan.entry_pc)
        {
            let mut trace = self
                .hotness
                .remove(index)
                .expect("hotness index must exist");
            if trace.guard_bytes != plan.guard_bytes() {
                trace.guard_bytes = plan.guard_bytes().to_vec();
                trace.hits = 1;
                trace.evictions = 0;
                trace.blocked = false;
            } else if !trace.blocked {
                trace.hits = trace.hits.saturating_add(1);
            }
            let should_compile = !trace.blocked && trace.hits >= COMPILE_HIT_THRESHOLD;
            self.hotness.push_front(trace);
            return should_compile;
        }

        self.hotness.push_front(TraceHotness {
            entry_pc: plan.entry_pc,
            guard_bytes: plan.guard_bytes().to_vec(),
            hits: 1,
            evictions: 0,
            blocked: false,
        });
        while self.hotness.len() > MAX_TRACE_HOTNESS_ENTRIES {
            self.hotness.pop_back();
        }
        false
    }

    fn record_evicted_trace(&mut self, cached: &CachedTrace) {
        if let Some(trace) = self.hotness.iter_mut().find(|trace| {
            trace.entry_pc == cached.entry_pc && trace.guard_bytes == cached.guard_bytes
        }) {
            trace.evictions = trace.evictions.saturating_add(1);
            trace.blocked |= trace.evictions >= MAX_TRACE_EVICTIONS;
            return;
        }

        self.hotness.push_front(TraceHotness {
            entry_pc: cached.entry_pc,
            guard_bytes: cached.guard_bytes.clone(),
            hits: COMPILE_HIT_THRESHOLD,
            evictions: 1,
            blocked: false,
        });
        while self.hotness.len() > MAX_TRACE_HOTNESS_ENTRIES {
            self.hotness.pop_back();
        }
    }

    fn run_cached(&mut self, index: usize, input: TraceInput) -> Option<TraceRun> {
        let cached = self.cache.remove(index)?;
        if cached.plan.cycle_upper_bound() > input.max_cycles {
            self.cache.push_front(cached);
            return None;
        }

        let mut state = TraceCpuState::new(
            cached.entry_pc,
            input.a,
            input.x,
            input.y,
            input.sp,
            input.p,
        );
        cached.trace.run(&mut state);
        let result = TraceRun {
            pc_to_fetch: state.pc as u16,
            a: state.a as u8,
            x: state.x as u8,
            y: state.y as u8,
            sp: state.sp as u8,
            p: state.p as u8,
            cycles: state.cycles,
        };
        self.cache.push_front(cached);
        Some(result)
    }
}

fn branch_cycles(branch_pc: u16, target_pc: u16, taken: bool) -> u8 {
    if !taken {
        2
    } else if (branch_pc.wrapping_add(2) & 0xFF00) == (target_pc & 0xFF00) {
        3
    } else {
        4
    }
}

fn emit_op(
    builder: &mut FunctionBuilder<'_>,
    op: TraceOp,
    a: &mut cranelift_codegen::ir::Value,
    x: &mut cranelift_codegen::ir::Value,
    y: &mut cranelift_codegen::ir::Value,
    sp: &mut cranelift_codegen::ir::Value,
    p: &mut cranelift_codegen::ir::Value,
) {
    match op {
        TraceOp::Dex => {
            let decremented = builder.ins().iadd_imm(*x, -1);
            *x = mask_u8(builder, decremented);
            *p = set_nz(builder, *p, *x);
        }
        TraceOp::Dey => {
            let decremented = builder.ins().iadd_imm(*y, -1);
            *y = mask_u8(builder, decremented);
            *p = set_nz(builder, *p, *y);
        }
        TraceOp::Inx => {
            let incremented = builder.ins().iadd_imm(*x, 1);
            *x = mask_u8(builder, incremented);
            *p = set_nz(builder, *p, *x);
        }
        TraceOp::Iny => {
            let incremented = builder.ins().iadd_imm(*y, 1);
            *y = mask_u8(builder, incremented);
            *p = set_nz(builder, *p, *y);
        }
        TraceOp::Tax => {
            *x = mask_u8(builder, *a);
            *p = set_nz(builder, *p, *x);
        }
        TraceOp::Tay => {
            *y = mask_u8(builder, *a);
            *p = set_nz(builder, *p, *y);
        }
        TraceOp::Txa => {
            *a = mask_u8(builder, *x);
            *p = set_nz(builder, *p, *a);
        }
        TraceOp::Tya => {
            *a = mask_u8(builder, *y);
            *p = set_nz(builder, *p, *a);
        }
        TraceOp::Tsx => {
            *x = mask_u8(builder, *sp);
            *p = set_nz(builder, *p, *x);
        }
        TraceOp::Txs => {
            *sp = mask_u8(builder, *x);
        }
        TraceOp::LdaImm(value) => {
            *a = iconst(builder, i64::from(value));
            *p = set_nz(builder, *p, *a);
        }
        TraceOp::LdxImm(value) => {
            *x = iconst(builder, i64::from(value));
            *p = set_nz(builder, *p, *x);
        }
        TraceOp::LdyImm(value) => {
            *y = iconst(builder, i64::from(value));
            *p = set_nz(builder, *p, *y);
        }
        TraceOp::CmpImm(value) => {
            *p = compare_u8(builder, *p, *a, value);
        }
        TraceOp::CpxImm(value) => {
            *p = compare_u8(builder, *p, *x, value);
        }
        TraceOp::CpyImm(value) => {
            *p = compare_u8(builder, *p, *y, value);
        }
        TraceOp::Clc => {
            *p = clear_flag(builder, *p, RegisterP::CARRY.bits());
        }
        TraceOp::Sec => {
            *p = set_flag(builder, *p, RegisterP::CARRY.bits());
        }
        TraceOp::Cld => {
            *p = clear_flag(builder, *p, RegisterP::DECIMAL.bits());
        }
        TraceOp::Sed => {
            *p = set_flag(builder, *p, RegisterP::DECIMAL.bits());
        }
        TraceOp::Clv => {
            *p = clear_flag(builder, *p, RegisterP::OVERFLOW.bits());
        }
        TraceOp::Nop => {}
    }
}

fn emit_branch_condition(
    builder: &mut FunctionBuilder<'_>,
    condition: BranchCondition,
    p: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    match condition {
        BranchCondition::Plus => flag_is_clear(builder, p, RegisterP::NEGATIVE.bits()),
        BranchCondition::Minus => flag_is_set(builder, p, RegisterP::NEGATIVE.bits()),
        BranchCondition::OverflowClear => flag_is_clear(builder, p, RegisterP::OVERFLOW.bits()),
        BranchCondition::OverflowSet => flag_is_set(builder, p, RegisterP::OVERFLOW.bits()),
        BranchCondition::CarryClear => flag_is_clear(builder, p, RegisterP::CARRY.bits()),
        BranchCondition::CarrySet => flag_is_set(builder, p, RegisterP::CARRY.bits()),
        BranchCondition::NotZero => flag_is_clear(builder, p, RegisterP::ZERO.bits()),
        BranchCondition::Equal => flag_is_set(builder, p, RegisterP::ZERO.bits()),
    }
}

fn compare_u8(
    builder: &mut FunctionBuilder<'_>,
    p: cranelift_codegen::ir::Value,
    lhs: cranelift_codegen::ir::Value,
    rhs: u8,
) -> cranelift_codegen::ir::Value {
    let lhs = mask_u8(builder, lhs);
    let rhs_value = iconst(builder, i64::from(rhs));
    let carry = builder
        .ins()
        .icmp(IntCC::UnsignedGreaterThanOrEqual, lhs, rhs_value);
    let p = set_flag_from_bool(builder, p, carry, RegisterP::CARRY.bits());
    let raw_diff = builder.ins().iadd_imm(lhs, -i64::from(rhs));
    let diff = mask_u8(builder, raw_diff);
    set_nz(builder, p, diff)
}

fn load_i64(
    builder: &mut FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    offset: i32,
) -> cranelift_codegen::ir::Value {
    builder
        .ins()
        .load(types::I64, MemFlags::trusted(), base, offset)
}

fn store_i64(
    builder: &mut FunctionBuilder<'_>,
    base: cranelift_codegen::ir::Value,
    offset: i32,
    value: cranelift_codegen::ir::Value,
) {
    builder
        .ins()
        .store(MemFlags::trusted(), value, base, offset);
}

fn iconst(builder: &mut FunctionBuilder<'_>, value: i64) -> cranelift_codegen::ir::Value {
    builder.ins().iconst(types::I64, value)
}

fn mask_u8(
    builder: &mut FunctionBuilder<'_>,
    value: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    builder.ins().band_imm(value, 0xFF)
}

fn set_nz(
    builder: &mut FunctionBuilder<'_>,
    p: cranelift_codegen::ir::Value,
    value: cranelift_codegen::ir::Value,
) -> cranelift_codegen::ir::Value {
    let value = mask_u8(builder, value);
    let zero = builder.ins().icmp_imm(IntCC::Equal, value, 0);
    let p = set_flag_from_bool(builder, p, zero, RegisterP::ZERO.bits());
    let negative = builder.ins().band_imm(value, 0x80);
    let negative = builder.ins().icmp_imm(IntCC::NotEqual, negative, 0);
    set_flag_from_bool(builder, p, negative, RegisterP::NEGATIVE.bits())
}

fn set_flag_from_bool(
    builder: &mut FunctionBuilder<'_>,
    p: cranelift_codegen::ir::Value,
    condition: cranelift_codegen::ir::Value,
    flag: u8,
) -> cranelift_codegen::ir::Value {
    let cleared = clear_flag(builder, p, flag);
    let set = set_flag(builder, cleared, flag);
    builder.ins().select(condition, set, cleared)
}

fn set_flag(
    builder: &mut FunctionBuilder<'_>,
    p: cranelift_codegen::ir::Value,
    flag: u8,
) -> cranelift_codegen::ir::Value {
    builder.ins().bor_imm(p, i64::from(flag))
}

fn clear_flag(
    builder: &mut FunctionBuilder<'_>,
    p: cranelift_codegen::ir::Value,
    flag: u8,
) -> cranelift_codegen::ir::Value {
    builder.ins().band_imm(p, i64::from(!flag))
}

fn flag_is_set(
    builder: &mut FunctionBuilder<'_>,
    p: cranelift_codegen::ir::Value,
    flag: u8,
) -> cranelift_codegen::ir::Value {
    let masked = builder.ins().band_imm(p, i64::from(flag));
    builder.ins().icmp_imm(IntCC::NotEqual, masked, 0)
}

fn flag_is_clear(
    builder: &mut FunctionBuilder<'_>,
    p: cranelift_codegen::ir::Value,
    flag: u8,
) -> cranelift_codegen::ir::Value {
    let masked = builder.ins().band_imm(p, i64::from(flag));
    builder.ins().icmp_imm(IntCC::Equal, masked, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_from<'a>(base: u16, bytes: &'a [u8]) -> impl FnMut(u16) -> Option<u8> + 'a {
        move |address| bytes.get(usize::from(address.wrapping_sub(base))).copied()
    }

    #[test]
    fn decodes_multi_op_block_ending_in_bne_with_guard_bytes() {
        let bytes = [0xA2, 0x03, 0xCA, 0xD0, 0xFD, 0xEA];
        let plan = TracePlan::decode_block(0x8000, read_from(0x8000, &bytes)).unwrap();

        assert_eq!(plan.entry_pc, 0x8000);
        assert_eq!(plan.ops, vec![TraceOp::LdxImm(3), TraceOp::Dex]);
        assert_eq!(plan.guard_bytes(), &bytes[..5]);
        assert_eq!(plan.cycle_upper_bound(), 7);
        assert_eq!(
            plan.exit,
            TraceExit::Branch {
                condition: BranchCondition::NotZero,
                branch_pc: 0x8003,
                taken_pc: 0x8002,
                fallthrough_pc: 0x8005,
                taken_cycles: 3,
                fallthrough_cycles: 2,
            }
        );
    }

    #[test]
    fn rejects_block_when_branch_exit_fetch_is_not_peek_safe() {
        let bytes = [0xCA, 0xD0, 0xFD];

        assert!(TracePlan::decode_block(0x1FFD, read_from(0x1FFD, &bytes)).is_none());
    }

    #[test]
    fn cranelift_trace_executes_ldx_dex_bne_block() {
        let bytes = [0xA2, 0x03, 0xCA, 0xD0, 0xFD, 0xEA];
        let plan = TracePlan::decode_block(0x8000, read_from(0x8000, &bytes)).unwrap();
        let mut compiler = TraceCompiler::new();
        let trace = compiler.compile(&plan).unwrap();
        let mut state = TraceCpuState::new(0x8000, 0, 0, 0x44, 0xFD, RegisterP::RESERVED.bits());

        trace.run(&mut state);

        assert_eq!(state.a, 0);
        assert_eq!(state.x, 2);
        assert_eq!(state.y, 0x44);
        assert_eq!(state.sp, 0xFD);
        assert_eq!(state.pc, 0x8002);
        assert_eq!(state.cycles, 7);
        assert_eq!(state.p & u64::from(RegisterP::ZERO.bits()), 0);
        assert_eq!(state.p & u64::from(RegisterP::NEGATIVE.bits()), 0);
    }

    #[test]
    fn cranelift_trace_executes_compare_branch_behavior() {
        let bytes = [0xA9, 0x03, 0xC9, 0x03, 0xF0, 0x02, 0xEA, 0xEA, 0xEA];
        let plan = TracePlan::decode_block(0x8000, read_from(0x8000, &bytes)).unwrap();
        let mut compiler = TraceCompiler::new();
        let trace = compiler.compile(&plan).unwrap();
        let mut state = TraceCpuState::new(0x8000, 0, 0, 0, 0xFD, RegisterP::RESERVED.bits());

        trace.run(&mut state);

        assert_eq!(state.a, 3);
        assert_eq!(state.pc, 0x8008);
        assert_eq!(state.cycles, 7);
        assert_ne!(state.p & u64::from(RegisterP::CARRY.bits()), 0);
        assert_ne!(state.p & u64::from(RegisterP::ZERO.bits()), 0);
        assert_eq!(state.p & u64::from(RegisterP::NEGATIVE.bits()), 0);
    }

    #[test]
    fn trace_jit_revalidates_guard_bytes_before_running_cached_trace() {
        let mut bytes = [0xA2, 0x03, 0xCA, 0xD0, 0xFD, 0xEA];
        let mut jit = TraceJit::default();

        let mut result = None;
        for _ in 0..COMPILE_HIT_THRESHOLD {
            result = jit.run_block_trace(
                TraceInput {
                    entry_pc: 0x8000,
                    a: 0,
                    x: 0,
                    y: 0,
                    sp: 0xFD,
                    p: RegisterP::RESERVED.bits(),
                    max_cycles: u64::MAX,
                },
                read_from(0x8000, &bytes),
            );
        }
        let result = result.unwrap();
        assert_eq!(result.x, 2);

        bytes[2] = 0xEA;
        assert!(
            jit.run_block_trace(
                TraceInput {
                    entry_pc: 0x8000,
                    a: 0,
                    x: 0,
                    y: 0,
                    sp: 0xFD,
                    p: RegisterP::RESERVED.bits(),
                    max_cycles: u64::MAX,
                },
                read_from(0x8000, &bytes),
            )
            .is_none()
        );
    }

    #[test]
    fn trace_jit_respects_cycle_budget() {
        let bytes = [0xA2, 0x03, 0xCA, 0xD0, 0xFD, 0xEA];
        let mut jit = TraceJit::default();

        assert!(
            jit.run_block_trace(
                TraceInput {
                    entry_pc: 0x8000,
                    a: 0,
                    x: 0,
                    y: 0,
                    sp: 0xFD,
                    p: RegisterP::RESERVED.bits(),
                    max_cycles: 6,
                },
                read_from(0x8000, &bytes),
            )
            .is_none()
        );
        let mut result = None;
        for _ in 0..COMPILE_HIT_THRESHOLD {
            result = jit.run_block_trace(
                TraceInput {
                    entry_pc: 0x8000,
                    a: 0,
                    x: 0,
                    y: 0,
                    sp: 0xFD,
                    p: RegisterP::RESERVED.bits(),
                    max_cycles: 7,
                },
                read_from(0x8000, &bytes),
            );
        }
        assert_eq!(result.unwrap().cycles, 7);
    }
}

use super::RegisterP;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{AbiParam, InstBuilder, MemFlags, UserFuncName, types};
use cranelift_codegen::isa::CallConv;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};
use std::marker::PhantomData;
use std::mem;

const STATE_X_OFFSET: i32 = mem::offset_of!(TraceCpuState, x) as i32;
const STATE_P_OFFSET: i32 = mem::offset_of!(TraceCpuState, p) as i32;
const STATE_PC_OFFSET: i32 = mem::offset_of!(TraceCpuState, pc) as i32;
const STATE_CYCLES_OFFSET: i32 = mem::offset_of!(TraceCpuState, cycles) as i32;

type TraceFn = unsafe extern "C" fn(*mut TraceCpuState);

#[repr(C)]
pub(crate) struct TraceCpuState {
    x: u64,
    p: u64,
    pc: u64,
    cycles: u64,
}

impl TraceCpuState {
    fn new(pc: u16, x: u8, p: u8) -> Self {
        Self {
            x: u64::from(x),
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
}

impl TracePlan {
    pub(crate) fn decode_counted_x_loop<F>(entry_pc: u16, mut read: F) -> Option<Self>
    where
        F: FnMut(u16) -> Option<u8>,
    {
        if read(entry_pc)? != 0xCA {
            return None;
        }
        let branch_pc = entry_pc.wrapping_add(1);
        if read(branch_pc)? != 0xD0 {
            return None;
        }
        let offset = read(branch_pc.wrapping_add(1))? as i8;
        let fallthrough_pc = branch_pc.wrapping_add(2);
        let taken_pc = fallthrough_pc.wrapping_add_signed(i16::from(offset));
        if taken_pc != entry_pc {
            return None;
        }

        Some(Self {
            entry_pc,
            ops: vec![TraceOp::Dex],
            exit: TraceExit::BranchLoop {
                condition: BranchCondition::NotZero,
                branch_pc,
                taken_pc,
                fallthrough_pc,
                taken_cycles: branch_cycles(branch_pc, taken_pc, true),
                fallthrough_cycles: branch_cycles(branch_pc, fallthrough_pc, false),
            },
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TraceOp {
    Dex,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum TraceExit {
    BranchLoop {
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
    NotZero,
}

pub(crate) struct CompiledTrace<'compiler> {
    func: TraceFn,
    _module: PhantomData<&'compiler JITModule>,
}

impl CompiledTrace<'_> {
    fn run(&self, state: &mut TraceCpuState) {
        // SAFETY: TraceCompiler creates functions with exactly this ABI and the
        // generated IR only touches the supplied TraceCpuState.
        unsafe { (self.func)(state) }
    }
}

pub(crate) struct TraceCompiler {
    module: JITModule,
    next_func_id: u32,
}

impl TraceCompiler {
    pub(crate) fn new() -> Self {
        let builder = JITBuilder::new(cranelift_module::default_libcall_names())
            .expect("Cranelift JIT builder should initialize");
        Self {
            module: JITModule::new(builder),
            next_func_id: 0,
        }
    }

    pub(crate) fn compile<'compiler>(
        &'compiler mut self,
        plan: &TracePlan,
    ) -> Option<CompiledTrace<'compiler>> {
        match (&plan.ops[..], &plan.exit) {
            (
                [TraceOp::Dex],
                TraceExit::BranchLoop {
                    condition: BranchCondition::NotZero,
                    fallthrough_pc,
                    taken_cycles,
                    fallthrough_cycles,
                    ..
                },
            ) => self.compile_dex_bne_loop(*fallthrough_pc, *taken_cycles, *fallthrough_cycles),
            _ => None,
        }
    }

    fn compile_dex_bne_loop(
        &mut self,
        fallthrough_pc: u16,
        taken_cycles: u8,
        fallthrough_cycles: u8,
    ) -> Option<CompiledTrace<'_>> {
        let pointer_type = self.module.target_config().pointer_type();
        let mut ctx = self.module.make_context();
        ctx.func.signature.call_conv = CallConv::triple_default(self.module.isa().triple());
        ctx.func.signature.params.push(AbiParam::new(pointer_type));
        ctx.func.name = UserFuncName::user(0, self.next_func_id);
        let name = format!("trace_jit_x_loop_{}", self.next_func_id);
        self.next_func_id = self.next_func_id.wrapping_add(1);
        let func_id = self
            .module
            .declare_function(&name, Linkage::Local, &ctx.func.signature)
            .ok()?;

        let mut builder_context = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_context);
        let entry = builder.create_block();
        let loop_block = builder.create_block();
        let exit = builder.create_block();
        builder.append_block_params_for_function_params(entry);
        let loop_x = builder.append_block_param(loop_block, types::I64);
        let loop_p = builder.append_block_param(loop_block, types::I64);
        let loop_cycles = builder.append_block_param(loop_block, types::I64);
        let exit_x = builder.append_block_param(exit, types::I64);
        let exit_p = builder.append_block_param(exit, types::I64);
        let exit_cycles = builder.append_block_param(exit, types::I64);

        builder.switch_to_block(entry);
        let state_ptr = builder.block_params(entry)[0];
        let x = load_i64(&mut builder, state_ptr, STATE_X_OFFSET);
        let p = load_i64(&mut builder, state_ptr, STATE_P_OFFSET);
        let cycles = load_i64(&mut builder, state_ptr, STATE_CYCLES_OFFSET);
        builder
            .ins()
            .jump(loop_block, &[x.into(), p.into(), cycles.into()]);

        builder.switch_to_block(loop_block);
        let x = builder.ins().iadd_imm(loop_x, -1);
        let x = mask_u8(&mut builder, x);
        let p = set_nz(&mut builder, loop_p, x);
        let cycles = builder.ins().iadd_imm(loop_cycles, 2);
        let zero = builder.ins().icmp_imm(IntCC::Equal, x, 0);
        let taken_cycles = builder.ins().iadd_imm(cycles, i64::from(taken_cycles));
        builder.ins().brif(
            zero,
            exit,
            &[x.into(), p.into(), cycles.into()],
            loop_block,
            &[x.into(), p.into(), taken_cycles.into()],
        );

        builder.switch_to_block(exit);
        let cycles = builder
            .ins()
            .iadd_imm(exit_cycles, i64::from(fallthrough_cycles));
        store_i64(&mut builder, state_ptr, STATE_X_OFFSET, exit_x);
        store_i64(&mut builder, state_ptr, STATE_P_OFFSET, exit_p);
        let fallthrough_pc = iconst(&mut builder, i64::from(fallthrough_pc));
        store_i64(&mut builder, state_ptr, STATE_PC_OFFSET, fallthrough_pc);
        store_i64(&mut builder, state_ptr, STATE_CYCLES_OFFSET, cycles);
        builder.ins().return_(&[]);
        builder.seal_all_blocks();
        builder.finalize();

        self.module.define_function(func_id, &mut ctx).ok()?;
        self.module.clear_context(&mut ctx);
        self.module.finalize_definitions().ok()?;
        let code = self.module.get_finalized_function(func_id);
        Some(CompiledTrace {
            // SAFETY: The finalized Cranelift function was declared and built
            // with the same ABI as TraceFn.
            func: unsafe { mem::transmute::<*const u8, TraceFn>(code) },
            _module: PhantomData,
        })
    }
}

impl Default for TraceCompiler {
    fn default() -> Self {
        Self::new()
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
    let cleared = builder.ins().band_imm(p, i64::from(!flag & 0xFF));
    let set = builder.ins().bor_imm(cleared, i64::from(flag));
    builder.ins().select(condition, set, cleared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_x_counted_branch_loop_trace() {
        let bytes = [0xCA, 0xD0, 0xFD];
        let plan = TracePlan::decode_counted_x_loop(0x8000, |address| {
            bytes.get(usize::from(address - 0x8000)).copied()
        })
        .unwrap();

        assert_eq!(plan.entry_pc, 0x8000);
        assert_eq!(plan.ops, vec![TraceOp::Dex]);
        assert_eq!(
            plan.exit,
            TraceExit::BranchLoop {
                condition: BranchCondition::NotZero,
                branch_pc: 0x8001,
                taken_pc: 0x8000,
                fallthrough_pc: 0x8003,
                taken_cycles: 3,
                fallthrough_cycles: 2,
            }
        );
    }

    #[test]
    fn cranelift_trace_executes_counted_x_loop() {
        let bytes = [0xCA, 0xD0, 0xFD];
        let plan = TracePlan::decode_counted_x_loop(0x8000, |address| {
            bytes.get(usize::from(address - 0x8000)).copied()
        })
        .unwrap();
        let mut compiler = TraceCompiler::new();
        let trace = compiler.compile(&plan).unwrap();
        let mut state = TraceCpuState::new(0x8000, 3, RegisterP::RESERVED.bits());

        trace.run(&mut state);

        assert_eq!(state.x, 0);
        assert_eq!(state.pc, 0x8003);
        assert_eq!(state.cycles, 14);
        assert_eq!(state.p & u64::from(RegisterP::ZERO.bits()), 0x02);
        assert_eq!(state.p & u64::from(RegisterP::NEGATIVE.bits()), 0);
    }
}

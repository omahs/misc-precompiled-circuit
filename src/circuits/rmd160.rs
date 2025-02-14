// The constraint system matrix for rmd160.

use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{Chip, Layouter, Region, AssignedCell, Value},
    plonk::{
        Fixed, Advice, Column, ConstraintSystem,
        Error, Expression, Selector, VirtualCells
    },
    poly::Rotation,
};

use std::marker::PhantomData;
use crate::host::rmd160::{
    ROUNDS_OFFSET,
    PROUNDS_OFFSET,
    R, O, PR, PO,
    RMD160Atomic,
};
use crate::constant;

use crate::utils::{
    field_to_u64,
    u32_to_limbs,
    cell_to_u32,
    cell_to_value,
    cell_to_limbs,
};

pub struct RMD160Chip<F: FieldExt> {
    config: RMD160Config,
    _marker: PhantomData<F>,
}


struct GateCell{
    cell: [usize;3],
    name: String,
}

impl GateCell {
    fn adv(col: usize, row: usize, dbg: &str) -> GateCell {
        GateCell {
            cell: [0, col, row],
            name: String::from(dbg),
        }
    }
    fn fix(col: usize, row: usize, dbg: &str) -> GateCell {
        GateCell {
            cell: [1, col, row],
            name: String::from(dbg),
        }
    }
    fn sel(col: usize, row: usize, dbg: &str) -> GateCell {
        GateCell {
            cell: [2, col, row],
            name: String::from(dbg),
        }
    }
    fn to_string(&self) -> String { self.name.clone() }
}


/*
 * | h_sel | r_sel | col0| col1  | col2 | col3 | col4 | col5  | col6   |  fix0     |
 * | h_sel | r_sel | a   | b     | c    |  d   | x    | e     | c_next |  offset   |
 * |       |       | w0  | b0    | c0   |  d0  | r0   | w1_h  | w4_h   |  w1_r     |
 * |       |       | wb  | b1    | c1   |  d1  | r1   | w1_l  | w4_l   |  w1_rr    |
 * |       |       | wc  | b2    | c2   |  d2  | r2   | a_next| w2b    |           |
 * |       |       | w1  | b3    | c3   |  d3  | r3   |       | w2c    |           |
 * 
 */

/* All witness we need to fill the gate */
struct RoundWitness<F: FieldExt> {
    r: u32,  // atomic(b, c, d)
    w0: u32, // a + r + x + offset
    wb: F, // a + r + x + offset u64
    wc: u64, // wb - w0
    w1: u32, // w0 rorate_left w1_r
    w1_h: u32,  //w1 >> w1_r
    w1_l: u32,  //w1 % w1_r
    a_next: u32, // w1 + e
    w2b: F, // w1+e u64 
    w2c: u64, // w2b - a_next
    w4_h: u32, // c >> w4_r
    w4_l: u32, // c % w4_r
    c_next: u32, // c rotate_left 10
}


fn get_witnesses<F: FieldExt>(round: usize, rol: &[u32; 5], x: u32, shift: u32, offset:u32, pround: bool) -> RoundWitness<F> {
    let f = if pround {5 - round - 1} else { round };
    let r = u32::atomic(f, rol[1], rol[2], rol[3]);
    let w0 = r.wrapping_add(rol[0]).wrapping_add(x).wrapping_add(offset);
    let wb = F::from(r as u64) + F::from(rol[0] as u64) + F::from(x as u64) + F::from(offset as u64);
    let wc = (field_to_u64(&wb) - (w0 as u64)) >> 32;
    let w1 = w0.rotate_left(shift);
    let w1_h = w0 >> (32 - shift);
    let w1_l = w0 % (2u32.pow(32 - shift));
    let a_next = w1.wrapping_add(rol[4]);
    let w2b = F::from(w1 as u64) + F::from(rol[4] as u64);
    let w2c = (field_to_u64(&w2b) - (a_next as u64)) >> 32;
    let w4_h = rol[2] >> 22;
    let w4_l = rol[2] % (2u32.pow(22));
    let c_next = rol[2].rotate_left(10);

    //println!("round {}, shift {}, offset {} x {}", r, shift ,offset, x);
    //println!("w2c {}", w2c);

    RoundWitness {
        r, w0, wb, wc, w1, w1_h, w1_l, a_next, w2b, w2c, w4_h, w4_l, c_next
    }
}



struct RoundGate ();

impl RoundGate {
    fn hsel(i: usize) -> GateCell { GateCell::sel(0,0, format!("hsel{}", i).as_str()) }
    fn rsel(i: usize) -> GateCell { GateCell::sel(1,i, format!("hsel{}", i).as_str()) }
    fn offset() -> GateCell { GateCell::fix(0,0, "offset") }
    fn w1_r() -> GateCell { GateCell::fix(0, 1, "w1r") }
    fn w1_rr() -> GateCell { GateCell::fix(0, 2, "w1rr") }

    fn a() -> GateCell { GateCell::adv(0,0, "a") }
    fn w0() -> GateCell { GateCell::adv(0,1, "w0") }
    fn wb() -> GateCell { GateCell::adv(0,2, "wb") }
    fn wc() -> GateCell { GateCell::adv(0,3, "wc") }
    fn w1() -> GateCell { GateCell::adv(0,4, "w1") }

    fn b() -> GateCell { GateCell::adv(1,0, "b") }
    fn c() -> GateCell { GateCell::adv(2,0, "c") }
    fn d() -> GateCell { GateCell::adv(3,0, "d") }
    fn x() -> GateCell { GateCell::adv(4,0, "x") }
    fn e() -> GateCell { GateCell::adv(5,0, "e") }

    fn blimb(i: usize) -> GateCell { GateCell::adv(1,i+1, format!("blimb{}",i).as_str()) }
    fn climb(i: usize) -> GateCell { GateCell::adv(2,i+1, format!("climb{}",i).as_str()) }
    fn dlimb(i: usize) -> GateCell { GateCell::adv(3,i+1, format!("dlimb{}",i).as_str()) }
    fn rlimb(i: usize) -> GateCell { GateCell::adv(4,i+1, format!("rlimb{}",i).as_str()) }
    fn w1_h() -> GateCell { GateCell::adv(5,1, "w1h") }
    fn w1_l() -> GateCell { GateCell::adv(5,2, "w1l") }
    fn a_next() -> GateCell { GateCell::adv(5,3, "anext") }
    fn c_next() -> GateCell { GateCell::adv(6,0, "cnext") }
    fn w4_h() -> GateCell { GateCell::adv(6,1, "w4h") }
    fn w4_l() -> GateCell { GateCell::adv(6,2, "w4l") }
    fn w2b() -> GateCell { GateCell::adv(6,3, "w2b") }
    fn w2c() -> GateCell { GateCell::adv(6,4, "w2c") }
}



/* Compress sum gate layout
 * | h_sel | r_sel | col0| col1  | col2 | col3 | col4 | col5  | col6 | -- |
 * | h_sel | r_sel | a   | b1    | c2   | sum1 | ca1  | bnew  |      |    |
 * |       |       | b   | c1    | d2   | sum2 | ca2  | cnew  |      |    |
 * |       |       | c   | d1    | e2   | sum3 | ca3  | dnew  |      |    |
 * |       |       | d   | e1    | a2   | sum4 | ca4  | enew  |      |    |
 * |       |       | e   | a1    | b2   | sum5 | ca5  | anew  |      |    |
 * 
 */

struct CompressGate ();

impl CompressGate {
    fn rsel(i: usize) -> GateCell { GateCell::sel(1,i, format!("hsel{}", i).as_str()) }
    fn a() -> GateCell { GateCell::adv(0, 0, "a") }
    fn b() -> GateCell { GateCell::adv(0, 1, "b") }
    fn c() -> GateCell { GateCell::adv(0, 2, "c") }
    fn d() -> GateCell { GateCell::adv(0, 3, "d") }
    fn e() -> GateCell { GateCell::adv(0, 4, "e") }

    fn b1() -> GateCell { GateCell::adv(1, 0, "b1") }
    fn c1() -> GateCell { GateCell::adv(1, 1, "c1") }
    fn d1() -> GateCell { GateCell::adv(1, 2, "d1") }
    fn e1() -> GateCell { GateCell::adv(1, 3, "e1") }
    fn a1() -> GateCell { GateCell::adv(1, 4, "a1") }

    fn c2() -> GateCell { GateCell::adv(2, 0, "c2") }
    fn d2() -> GateCell { GateCell::adv(2, 1, "d2") }
    fn e2() -> GateCell { GateCell::adv(2, 2, "e2") }
    fn a2() -> GateCell { GateCell::adv(2, 3, "a2") }
    fn b2() -> GateCell { GateCell::adv(2, 4, "b2") }

    fn sum0() -> GateCell { GateCell::adv(3, 0, "sum0") }
    fn sum1() -> GateCell { GateCell::adv(3, 1, "sum1") }
    fn sum2() -> GateCell { GateCell::adv(3, 2, "sum2") }
    fn sum3() -> GateCell { GateCell::adv(3, 3, "sum3") }
    fn sum4() -> GateCell { GateCell::adv(3, 4, "sum4") }

    fn ca0() -> GateCell { GateCell::adv(4, 0, "ca0") }
    fn ca1() -> GateCell { GateCell::adv(4, 1, "ca1") }
    fn ca2() -> GateCell { GateCell::adv(4, 2, "ca2") }
    fn ca3() -> GateCell { GateCell::adv(4, 3, "ca3") }
    fn ca4() -> GateCell { GateCell::adv(4, 4, "ca4") }

    fn bnew() -> GateCell { GateCell::adv(4, 0, "bnew") }
    fn cnew() -> GateCell { GateCell::adv(4, 1, "cnew") }
    fn dnew() -> GateCell { GateCell::adv(4, 2, "dnew") }
    fn enew() -> GateCell { GateCell::adv(4, 3, "enew") }
    fn anew() -> GateCell { GateCell::adv(4, 4, "anew") }
}

#[derive(Clone, Debug)]
pub struct RMD160Config {
    witness: [Column<Advice>; 7],
    selector: [Selector; 2],
    fixed: [Column<Fixed>; 1],
}

impl RMD160Config {
    fn get_expr<F:FieldExt>(&self, meta: &mut VirtualCells<F>, gate_cell: GateCell) -> Expression<F> {
        let cell = gate_cell.cell;
        if cell[0] == 0 { // advice
            meta.query_advice(self.witness[cell[1]], Rotation(cell[2] as i32))
        } else if cell[0] == 1 { // fix
            meta.query_fixed(self.fixed[cell[1]], Rotation(cell[2] as i32))
        } else { // selector
            meta.query_selector(self.selector[cell[1]])
        }
    }
}

impl<F: FieldExt> Chip<F> for RMD160Chip<F> {
    type Config = RMD160Config;
    type Loaded = ();

    fn config(&self) -> &Self::Config {
        &self.config
    }

    fn loaded(&self) -> &Self::Loaded {
        &()
    }
}

impl<F: FieldExt> RMD160Chip<F> {
    pub fn new(config: RMD160Config) -> Self {
        RMD160Chip {
            config,
            _marker: PhantomData,
        }
    }

    pub fn configure(cs: &mut ConstraintSystem<F>) -> RMD160Config {
        let witness= [0; 7]
                .map(|_|cs.advice_column());
        let fixed= [0; 1]
                .map(|_|cs.fixed_column());
        let selector= [0; 2]
                .map(|_|cs.selector());
        witness.map(|x| cs.enable_equality(x));

        let config = RMD160Config { fixed, selector, witness };

        cs.create_gate("sum with bound", |meta| {
            let mut sum_r = config.get_expr(meta, RoundGate::rlimb(0));
            for i in 1..4 {
                let limb = config.get_expr(meta, RoundGate::rlimb(i));
                sum_r = sum_r + limb * F::from(1u64 << (8*i));
            }
            let w0 = config.get_expr(meta, RoundGate::w0());
            let wb = config.get_expr(meta, RoundGate::wb());
            let wc = config.get_expr(meta, RoundGate::wc());
            let a = config.get_expr(meta, RoundGate::a());
            let x = config.get_expr(meta, RoundGate::x());
            let offset = config.get_expr(meta, RoundGate::offset());
            let hsel = config.get_expr(meta, RoundGate::hsel(0));
            vec![
                (wb.clone() - sum_r - a - x - offset) * hsel.clone(),
                //(wc.clone()*(wc.clone() - constant!(F::one()))) * hsel.clone(),
                (w0 + wc * F::from(1u64 << 32) - wb) * hsel,
            ] 
        });

        cs.create_gate("sum with w1 rol4", |meta| {
            let a_next = config.get_expr(meta, RoundGate::a_next());
            let hsel = config.get_expr(meta, RoundGate::hsel(0));
            let w1 = config.get_expr(meta, RoundGate::w1());
            let w2b = config.get_expr(meta, RoundGate::w2b());
            let w2c = config.get_expr(meta, RoundGate::w2c());
            let e = config.get_expr(meta, RoundGate::e());
            vec![
                (w2b.clone() - w1 - e) * hsel.clone(),
                (w2c.clone()*(w2c.clone() - constant!(F::one()))) * hsel.clone(),
                (a_next + w2c * F::from(1u64 << 32) - w2b) * hsel,
            ]
        });

        cs.create_gate("limbs sum", |meta| {
            let hsel = config.get_expr(meta, RoundGate::hsel(0));

            let b = config.get_expr(meta, RoundGate::b());
            let mut sum_b = config.get_expr(meta, RoundGate::blimb(0));
            for i in 1..4 {
                let limb = config.get_expr(meta, RoundGate::blimb(i));
                sum_b = sum_b + limb * F::from(1u64 << (8*i));
            }

            let c = config.get_expr(meta, RoundGate::c());
            let mut sum_c = config.get_expr(meta, RoundGate::climb(0));
            for i in 1..4 {
                let limb = config.get_expr(meta, RoundGate::climb(i));
                sum_c = sum_c + limb * F::from(1u64 << (8*i));
            }

            let d = config.get_expr(meta, RoundGate::d());
            let mut sum_d = config.get_expr(meta, RoundGate::dlimb(0));
            for i in 1..4 {
                let limb = config.get_expr(meta, RoundGate::dlimb(i));
                sum_d = sum_d + limb * F::from(1u64 << (8*i));
            }

            vec![
                (sum_b - b) * hsel.clone(),
                (sum_c - c) * hsel.clone(),
                (sum_d - d) * hsel.clone(),
            ]
        });

        cs.create_gate("c rotate", |meta| {
            let hsel = config.get_expr(meta, RoundGate::hsel(0));

            let c = config.get_expr(meta, RoundGate::c());
            let c_next = config.get_expr(meta, RoundGate::c_next());
            let w4l = config.get_expr(meta, RoundGate::w4_l());
            let w4h = config.get_expr(meta, RoundGate::w4_h());

            vec![
                (w4h.clone() * constant!(F::from(1u64 << 22)) + w4l.clone() - c.clone()) * hsel.clone(),
                (w4l * constant!(F::from(1u64 << 10)) + w4h - c_next.clone()) * hsel.clone(),
            ]
        });

        cs.create_gate("w0 rotate", |meta| {
            let hsel = config.get_expr(meta, RoundGate::hsel(0));
            let w0 = config.get_expr(meta, RoundGate::w0());
            let w1 = config.get_expr(meta, RoundGate::w1());
            let w1l = config.get_expr(meta, RoundGate::w1_l());
            let w1h = config.get_expr(meta, RoundGate::w1_h());
            let shift = config.get_expr(meta, RoundGate::w1_r());
            let shift2 = config.get_expr(meta, RoundGate::w1_rr());

            vec![
                (w1h.clone() * shift2.clone() + w1l.clone() - w0) * hsel.clone(),
                (w1l * shift + w1h - w1) * hsel.clone(),
            ]
        });

        config
    }

    fn assign_cell(
        &self,
        region: &mut Region<F>,
        start_offset: usize,
        gate_cell: GateCell, 
        value: F,
    ) -> Result<AssignedCell<F, F>, Error> {
        let cell = gate_cell.cell;
        //println!("Assign Cell at {} {} {:?}", start_offset, gate_cell.name, value);
        if cell[0] == 0 { // advice
            region.assign_advice(
                || format!("assign cell"),
                self.config.witness[cell[1]],
                start_offset + cell[2],
                || Value::known(value)
            )
        } else if cell[0] == 1 { // fix
            region.assign_fixed(
                || format!("assign cell"),
                self.config.fixed[cell[1]],
                start_offset + cell[2],
                || Value::known(value)
            )
        } else { // selector
            unreachable!()
        }
    }

    fn enable_selector(
        &self,
        region: &mut Region<F>,
        start_offset: usize,
        gate_cell: GateCell,
        value: F,
    ) -> Result<(), Error> {
        assert!(gate_cell.cell[0] == 2);
        self.config.selector[gate_cell.cell[1]].enable(region, start_offset + gate_cell.cell[2])
    }


    fn bind_cell(
        &self,
        region: &mut Region<F>,
        start_offset: usize,
        cell: GateCell,
        value: &AssignedCell<F, F>,
    ) -> Result<AssignedCell<F, F>, Error> {
        let f = cell_to_value(value);
        let cell = self.assign_cell(region, start_offset,cell, f)?;
        region.constrain_equal(cell.cell(), value.cell())?;
        Ok(cell)
    }

    fn assign_next(
        &self,
        region: &mut Region<F>,
        start_offset: usize,
        previous: &[AssignedCell<F, F>; 5],
        input: &AssignedCell<F, F>,
        round: usize,
        index: usize,
        shift: &[[u32; 16]; 5],
        offset: &[u32; 5],
        pround: bool,
    ) -> Result<[AssignedCell<F, F>; 5], Error> {
        //println!("rol: {:?}", previous.clone().map(|x| cell_to_u32(&x)));
        self.bind_cell(region, start_offset, RoundGate::a(), &previous[0])?;
        let b = self.bind_cell(region, start_offset, RoundGate::b(), &previous[1])?;
        self.bind_cell(region, start_offset, RoundGate::c(), &previous[2])?;
        let d = self.bind_cell(region, start_offset, RoundGate::d(), &previous[3])?;
        let e = self.bind_cell(region, start_offset, RoundGate::e(), &previous[4])?;

        self.bind_cell(region, start_offset, RoundGate::x(), &input)?;

        self.assign_cell(region, start_offset, RoundGate::w1_r(), F::from(1u64 << shift[round][index]))?;
        self.assign_cell(region, start_offset, RoundGate::w1_rr(), F::from(1u64 << (32 - shift[round][index])))?;

        let blimbs = cell_to_limbs(&previous[1]);
        for i in 0..4 {
            self.assign_cell(region, start_offset, RoundGate::blimb(i), blimbs[i])?;
        }

        let climbs = cell_to_limbs(&previous[2]);
        for i in 0..4 {
            self.assign_cell(region, start_offset, RoundGate::climb(i), climbs[i])?;
        }

        let dlimbs = cell_to_limbs(&previous[3]);
        for i in 0..4 {
            self.assign_cell(region, start_offset, RoundGate::dlimb(i), dlimbs[i])?;
        }

        let rol = previous.into_iter()
            .map(|c| {
                cell_to_u32(c)
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap();

        let witness = get_witnesses(round, &rol, cell_to_u32(&input), shift[round][index], offset[round], pround);
        //self.assign_cell(region, start_offset, RoundGate::r(), F::from(witness.r as u64));
        //
        self.assign_cell(region, start_offset, RoundGate::offset(), F::from(offset[round] as u64))?;
        let rlimbs = u32_to_limbs(witness.r);

        let mut sum_r = rlimbs[0];
        for i in 1..4 {
            sum_r = sum_r + rlimbs[i] * F::from(1u64 << (8*i));
        }

        assert!(sum_r == F::from(witness.r as u64));

        assert!(witness.w2b == F::from(witness.w1 as u64) + F::from(cell_to_u32(&previous[4]) as u64));
        assert!(witness.wb == F::from(witness.r as u64) + F::from(cell_to_u32(&previous[0]) as u64)
                + F::from(cell_to_u32(&input) as u64) + F::from(offset[round] as u64));
        for i in 0..4 {
            self.assign_cell(region, start_offset, RoundGate::rlimb(i), rlimbs[i])?;
        }
        self.assign_cell(region, start_offset, RoundGate::w0(), F::from(witness.w0 as u64))?;
        self.assign_cell(region, start_offset, RoundGate::wb(), witness.wb)?;
        self.assign_cell(region, start_offset, RoundGate::wc(), F::from(witness.wc as u64))?;
        self.assign_cell(region, start_offset, RoundGate::w1(), F::from(witness.w1 as u64))?;
        self.assign_cell(region, start_offset, RoundGate::w1_h(), F::from(witness.w1_h as u64))?;
        self.assign_cell(region, start_offset, RoundGate::w1_l(), F::from(witness.w1_l as u64))?;
        self.assign_cell(region, start_offset, RoundGate::w4_h(), F::from(witness.w4_h as u64))?;
        self.assign_cell(region, start_offset, RoundGate::w4_l(),F::from(witness.w4_l as u64))?;
        self.assign_cell(region, start_offset, RoundGate::w2b(),witness.w2b)?;
        self.assign_cell(region, start_offset, RoundGate::w2c(),F::from(witness.w2c as u64))?;
        self.enable_selector(region, start_offset, RoundGate::hsel(0), F::one())?;
        let a = self.assign_cell(region, start_offset, RoundGate::a_next(), F::from(witness.a_next as u64))?;
        let c = self.assign_cell(region, start_offset, RoundGate::c_next(), F::from(witness.c_next as u64))?;
        Ok([e, a, b, c, d])
    }

    fn rotate_inputs(
        &self,
        inputs: &[AssignedCell<F, F>; 16],
        round_shift: [usize; 16],
    ) -> [AssignedCell<F, F>; 16] {
        round_shift.map(|i| inputs[i].clone())
    }

    pub fn assign_compress(
        &self,
        region: &mut Region<F>,
        start_offset: usize,
        r0: &[AssignedCell<F, F>; 5],
        r1: &[AssignedCell<F, F>; 5],
        r2: &[AssignedCell<F, F>; 5],
    ) -> Result<[AssignedCell<F, F>; 5], Error> {
        self.bind_cell(region, start_offset, CompressGate::a(), &r0[0])?;
        self.bind_cell(region, start_offset, CompressGate::b(), &r0[1])?;
        self.bind_cell(region, start_offset, CompressGate::c(), &r0[2])?;
        self.bind_cell(region, start_offset, CompressGate::d(), &r0[3])?;
        self.bind_cell(region, start_offset, CompressGate::e(), &r0[4])?;
        self.bind_cell(region, start_offset, CompressGate::a1(), &r1[0])?;
        self.bind_cell(region, start_offset, CompressGate::b1(), &r1[1])?;
        self.bind_cell(region, start_offset, CompressGate::c1(), &r1[2])?;
        self.bind_cell(region, start_offset, CompressGate::d1(), &r1[3])?;
        self.bind_cell(region, start_offset, CompressGate::e1(), &r1[4])?;
        self.bind_cell(region, start_offset, CompressGate::a2(), &r2[0])?;
        self.bind_cell(region, start_offset, CompressGate::b2(), &r2[1])?;
        self.bind_cell(region, start_offset, CompressGate::c2(), &r2[2])?;
        self.bind_cell(region, start_offset, CompressGate::d2(), &r2[3])?;
        self.bind_cell(region, start_offset, CompressGate::e2(), &r2[4])?;

        let anew = {
            let anew = cell_to_u32(&r0[0])
                .wrapping_add(cell_to_u32(&r1[1]))
                .wrapping_add(cell_to_u32(&r2[2]));
            let sum0 = cell_to_value(&r0[0])
                + cell_to_value(&r1[1])
                + cell_to_value(&r2[2]);
            let ca0 = (field_to_u64(&sum0) - anew as u64) >> 32;
            self.assign_cell(region, start_offset, CompressGate::sum0(), sum0)?;
            self.assign_cell(region, start_offset, CompressGate::ca0(), F::from(ca0))?;
            self.assign_cell(region, start_offset, CompressGate::anew(), F::from(anew as u64))?
        };

        let bnew = {
            let bnew = cell_to_u32(&r0[1])
                .wrapping_add(cell_to_u32(&r1[2]))
                .wrapping_add(cell_to_u32(&r2[3]));
            let sum1 = cell_to_value(&r0[1])
                + cell_to_value(&r1[2])
                + cell_to_value(&r2[3]);
            let ca1 = (field_to_u64(&sum1) - bnew as u64) >> 32;
            self.assign_cell(region, start_offset, CompressGate::sum1(), sum1)?;
            self.assign_cell(region, start_offset, CompressGate::ca1(), F::from(ca1))?;
            self.assign_cell(region, start_offset, CompressGate::bnew(), F::from(bnew as u64))?
        };

        let cnew = {
            let cnew = cell_to_u32(&r0[2])
                .wrapping_add(cell_to_u32(&r1[3]))
                .wrapping_add(cell_to_u32(&r2[4]));
            let sum2 = cell_to_value(&r0[2])
                + cell_to_value(&r1[3])
                + cell_to_value(&r2[4]);
            let ca2 = (field_to_u64(&sum2) - cnew as u64) >> 32;
            self.assign_cell(region, start_offset, CompressGate::sum2(), sum2)?;
            self.assign_cell(region, start_offset, CompressGate::ca0(), F::from(ca2))?;
            self.assign_cell(region, start_offset, CompressGate::cnew(), F::from(cnew as u64))?
        };

        let dnew = {
            let dnew = cell_to_u32(&r0[3])
                .wrapping_add(cell_to_u32(&r1[4]))
                .wrapping_add(cell_to_u32(&r2[0]));
            let sum3 = cell_to_value(&r0[3])
                + cell_to_value(&r1[4])
                + cell_to_value(&r2[0]);
            let ca3 = (field_to_u64(&sum3) - dnew as u64) >> 32;
            self.assign_cell(region, start_offset, CompressGate::sum3(), sum3)?;
            self.assign_cell(region, start_offset, CompressGate::ca3(), F::from(ca3))?;
            self.assign_cell(region, start_offset, CompressGate::dnew(), F::from(dnew as u64))?
        };

        let enew = {
            let enew = cell_to_u32(&r0[4])
                .wrapping_add(cell_to_u32(&r1[0]))
                .wrapping_add(cell_to_u32(&r2[1]));
            let sum4 = cell_to_value(&r0[4])
                + cell_to_value(&r1[0])
                + cell_to_value(&r2[1]);
            let ca4 = (field_to_u64(&sum4) - enew as u64) >> 32;
            self.assign_cell(region, start_offset, CompressGate::sum4(), sum4)?;
            self.assign_cell(region, start_offset, CompressGate::ca4(), F::from(ca4))?;
            self.assign_cell(region, start_offset, CompressGate::enew(), F::from(enew as u64))?
        };

        Ok([anew, bnew, cnew, dnew, enew])
    }


    pub fn assign_content(
        &self,
        layouter: &mut impl Layouter<F>,
        start_buf: &[AssignedCell<F, F>; 5],
        inputs: &[AssignedCell<F, F>; 16],
    ) -> Result<[AssignedCell<F, F>; 5], Error> {
        let r = layouter.assign_region(
            || "leaf layer",
            |mut region| {
                let mut r1 = start_buf.clone();
                let mut start_offset = 0;
                for round in 0..5 {
                    for index in 0..16 {
                        r1 = self.assign_next(
                            &mut region,
                            start_offset,
                            &r1,
                            &self.rotate_inputs(inputs, O[round])[index],
                            round,
                            index,
                            &R,
                            &ROUNDS_OFFSET,
                            false,
                        )?;
                        start_offset += 5;
                    }
                }
            println!("{} {} {} {} {}",
                cell_to_u32(&r1[0]),
                cell_to_u32(&r1[1]),
                cell_to_u32(&r1[2]),
                cell_to_u32(&r1[3]),
                cell_to_u32(&r1[4]),
            );


                let mut r2 = start_buf.clone();
                for round in 0..5 {
                    for index in 0..16 {
                        r2 = self.assign_next(
                            &mut region,
                            start_offset,
                            &r2,
                            &self.rotate_inputs(&inputs, PO[round])[index],
                            round,
                            index,
                            &PR,
                            &PROUNDS_OFFSET,
                            true
                        )?;
                        start_offset += 5;
                    }
                }
                self.assign_compress(&mut region, start_offset, start_buf, &r1, &r2)
            }
        )?;
        Ok(r)
    }
}



#[cfg(test)]
mod tests {
    use halo2_proofs::halo2curves::bn256::Fr;
    use halo2_proofs::dev::MockProver;

    use crate::utils::cell_to_u32;
    use halo2_proofs::{
        circuit::{Value, Chip, Layouter, AssignedCell, SimpleFloorPlanner},
        plonk::{
            Advice, Circuit, Column, ConstraintSystem, Error,
        },
    };

    use super::RMD160Chip;
    use super::RMD160Config;
    use crate::host::rmd160::H0;

    #[derive(Clone, Debug)]
    pub struct HelperChipConfig {
        limb: Column<Advice>
    }

    #[derive(Clone, Debug)]
    pub struct HelperChip {
        config: HelperChipConfig
    }

    impl Chip<Fr> for HelperChip {
        type Config = HelperChipConfig;
        type Loaded = ();

        fn config(&self) -> &Self::Config {
            &self.config
        }

        fn loaded(&self) -> &Self::Loaded {
            &()
        }
    }

    impl HelperChip {
        fn new(config: HelperChipConfig) -> Self {
            HelperChip{
                config,
            }
        }

        fn configure(cs: &mut ConstraintSystem<Fr>) -> HelperChipConfig {
            let limb= cs.advice_column();
            cs.enable_equality(limb);
            HelperChipConfig {
                limb,
            }
        }

        fn assign_w(
            &self,
            layouter: &mut impl Layouter<Fr>,
            inputs: &[u32; 5],
            offset: usize,
        ) -> Result<[AssignedCell<Fr, Fr>; 5], Error> {
            layouter.assign_region(
                || "leaf layer",
                |mut region| {
                    let mut r = vec![];
                    for round in 0..5 {
                        r.push(region.assign_advice(
                            || format!("assign w"),
                            self.config.limb,
                            offset + round,
                            || Value::known(Fr::from(inputs[round] as u64))
                        )?);
                    }
                    Ok(r.try_into().unwrap())
                }
            )
        }

        fn assign_inputs(
            &self,
            layouter: &mut impl Layouter<Fr>,
            inputs: &[Fr; 16],
            offset: usize
        ) -> Result<[AssignedCell<Fr, Fr>; 16], Error> {
            layouter.assign_region(
                || "leaf layer",
                |mut region| {
                    let mut r = vec![];
                    for i in 0..16 {
                        let cell = region.assign_advice(
                            || format!("assign input"),
                            self.config.limb,
                            offset + i,
                            || Value::known(inputs[i])
                        )?;
                        r.push(cell);
                    }
                    Ok(r.try_into().unwrap())
                }
            )
        }
    }

    #[derive(Clone, Debug, Default)]
    struct RMD160Circuit {
        inputs: [Fr; 16],
    }

    #[derive(Clone, Debug)]
    struct TestConfig {
        rmd160config: RMD160Config,
        helperconfig: HelperChipConfig,
    }

    impl Circuit<Fr> for RMD160Circuit {
        type Config = TestConfig;
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            Self::default()
        }

        fn configure(meta: &mut ConstraintSystem<Fr>) -> Self::Config {
            Self::Config {
               rmd160config: RMD160Chip::<Fr>::configure(meta),
               helperconfig: HelperChip::configure(meta)
            }
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<Fr>,
        ) -> Result<(), Error> {
            let rmd160chip = RMD160Chip::<Fr>::new(config.clone().rmd160config);
            let helperchip = HelperChip::new(config.clone().helperconfig);
            let w= helperchip.assign_w(&mut layouter, &H0, 0)?;
            let input = helperchip.assign_inputs(&mut layouter, &self.inputs, 0)?;
            let r = rmd160chip.assign_content(&mut layouter, &w, &input)?;
            println!("{} {} {} {} {}",
                cell_to_u32(&r[0]),
                cell_to_u32(&r[1]),
                cell_to_u32(&r[2]),
                cell_to_u32(&r[3]),
                cell_to_u32(&r[4]),
            );
            Ok(())
        }
    }


    #[test]
    fn test_rmd160_circuit() {
        let test_circuit = RMD160Circuit {inputs: [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16].map(|x| Fr::from(x as u64))} ;
        let prover = MockProver::run(16, &test_circuit, vec![]).unwrap();
        assert_eq!(prover.verify(), Ok(()));
    }
}



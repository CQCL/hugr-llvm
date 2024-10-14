; ModuleID = 'hugr-llvm'
source_filename = "hugr-llvm"

%RESULT = type opaque

@0 = private unnamed_addr constant [32 x i8] c"Non-finite number of half-turns\00", align 1
@prelude.panic_template = private unnamed_addr constant [34 x i8] c"Program panicked (signal %i): %s\0A\00", align 1
@1 = private unnamed_addr constant [2 x i8] c"1\00", align 1

define i16 @_hl.rx.1(i16 %0, { { double } } %1) {
alloca_block:
  %"0" = alloca i16, align 2
  %"2_0" = alloca i16, align 2
  %"2_1" = alloca { { double } }, align 8
  %"7_0" = alloca i16, align 2
  %"9_0" = alloca i16, align 2
  %"9_1" = alloca { { double } }, align 8
  %"03" = alloca i16, align 2
  %"32_0" = alloca { {} }, align 8
  %"30_0" = alloca i16, align 2
  %"31_0" = alloca { {} }, align 8
  %"29_0" = alloca { {} }, align 8
  %"14_0" = alloca { {} }, align 8
  %"12_0" = alloca double, align 8
  %"15_0" = alloca { { double } }, align 8
  %"16_0" = alloca double, align 8
  %"17_0" = alloca { i32, {}, { double } }, align 8
  %"18_0" = alloca double, align 8
  %"011" = alloca double, align 8
  %"23_0" = alloca { i32, i8* }, align 8
  %"24_0" = alloca double, align 8
  %"015" = alloca double, align 8
  %"26_0" = alloca double, align 8
  %"13_0" = alloca i16, align 2
  %"28_0" = alloca i16, align 2
  br label %entry_block

entry_block:                                      ; preds = %alloca_block
  store i16 %0, i16* %"2_0", align 2
  store { { double } } %1, { { double } }* %"2_1", align 8
  %"2_01" = load i16, i16* %"2_0", align 2
  %"2_12" = load { { double } }, { { double } }* %"2_1", align 8
  store i16 %"2_01", i16* %"9_0", align 2
  store { { double } } %"2_12", { { double } }* %"9_1", align 8
  br label %2

2:                                                ; preds = %entry_block
  %"9_04" = load i16, i16* %"9_0", align 2
  %"9_15" = load { { double } }, { { double } }* %"9_1", align 8
  store { {} } undef, { {} }* %"32_0", align 1
  store { {} } undef, { {} }* %"31_0", align 1
  store { {} } undef, { {} }* %"29_0", align 1
  store { {} } undef, { {} }* %"14_0", align 1
  store i16 %"9_04", i16* %"9_0", align 2
  store { { double } } %"9_15", { { double } }* %"9_1", align 8
  %"9_16" = load { { double } }, { { double } }* %"9_1", align 8
  %3 = extractvalue { { double } } %"9_16", 0
  %4 = extractvalue { double } %3, 0
  store double %4, double* %"12_0", align 8
  %"12_07" = load double, double* %"12_0", align 8
  %5 = insertvalue { double } undef, double %"12_07", 0
  %6 = insertvalue { { double } } poison, { double } %5, 0
  store { { double } } %6, { { double } }* %"15_0", align 8
  %"15_08" = load { { double } }, { { double } }* %"15_0", align 8
  %7 = extractvalue { { double } } %"15_08", 0
  %8 = extractvalue { double } %7, 0
  store double %8, double* %"16_0", align 8
  %"16_09" = load double, double* %"16_0", align 8
  %9 = fcmp oeq double %"16_09", 0x7FF0000000000000
  %10 = fcmp oeq double %"16_09", 0xFFF0000000000000
  %11 = fcmp uno double %"16_09", 0.000000e+00
  %12 = or i1 %9, %10
  %13 = or i1 %12, %11
  %14 = xor i1 %13, true
  %15 = insertvalue { double } undef, double %"16_09", 0
  %16 = insertvalue { i32, {}, { double } } { i32 1, {} poison, { double } poison }, { double } %15, 2
  %17 = select i1 %14, { i32, {}, { double } } %16, { i32, {}, { double } } { i32 0, {} undef, { double } poison }
  store { i32, {}, { double } } %17, { i32, {}, { double } }* %"17_0", align 8
  %"17_010" = load { i32, {}, { double } }, { i32, {}, { double } }* %"17_0", align 8
  %18 = extractvalue { i32, {}, { double } } %"17_010", 0
  switch i32 %18, label %19 [
    i32 1, label %21
  ]

19:                                               ; preds = %2
  %20 = extractvalue { i32, {}, { double } } %"17_010", 1
  br label %cond_18_case_0

21:                                               ; preds = %2
  %22 = extractvalue { i32, {}, { double } } %"17_010", 2
  %23 = extractvalue { double } %22, 0
  store double %23, double* %"015", align 8
  br label %cond_18_case_1

24:                                               ; preds = %28
  %"026" = load i16, i16* %"03", align 2
  store i16 %"026", i16* %"7_0", align 2
  %"7_027" = load i16, i16* %"7_0", align 2
  store i16 %"7_027", i16* %"0", align 2
  %"028" = load i16, i16* %"0", align 2
  ret i16 %"028"

cond_18_case_0:                                   ; preds = %19
  store { i32, i8* } { i32 1, i8* getelementptr inbounds ([32 x i8], [32 x i8]* @0, i32 0, i32 0) }, { i32, i8* }* %"23_0", align 8
  %"23_013" = load { i32, i8* }, { i32, i8* }* %"23_0", align 8
  %25 = extractvalue { i32, i8* } %"23_013", 0
  %26 = extractvalue { i32, i8* } %"23_013", 1
  %27 = call i32 (i8*, ...) @printf(i8* getelementptr inbounds ([34 x i8], [34 x i8]* @prelude.panic_template, i32 0, i32 0), i32 %25, i8* %26)
  call void @abort()
  store double 0.000000e+00, double* %"24_0", align 8
  %"24_014" = load double, double* %"24_0", align 8
  store double %"24_014", double* %"011", align 8
  br label %cond_exit_18

cond_18_case_1:                                   ; preds = %21
  %"016" = load double, double* %"015", align 8
  store double %"016", double* %"26_0", align 8
  %"26_017" = load double, double* %"26_0", align 8
  store double %"26_017", double* %"011", align 8
  br label %cond_exit_18

cond_exit_18:                                     ; preds = %cond_18_case_1, %cond_18_case_0
  %"012" = load double, double* %"011", align 8
  store double %"012", double* %"18_0", align 8
  %"9_018" = load i16, i16* %"9_0", align 2
  call void @__quantum__qis__h__body(i16 %"9_018")
  store i16 %"9_018", i16* %"13_0", align 2
  %"13_019" = load i16, i16* %"13_0", align 2
  %"18_020" = load double, double* %"18_0", align 8
  call void @__quantum__qis__rz__body(double %"18_020", i16 %"13_019")
  store i16 %"13_019", i16* %"28_0", align 2
  %"28_021" = load i16, i16* %"28_0", align 2
  call void @__quantum__qis__h__body(i16 %"28_021")
  store i16 %"28_021", i16* %"30_0", align 2
  %"32_022" = load { {} }, { {} }* %"32_0", align 1
  %"30_023" = load i16, i16* %"30_0", align 2
  store { {} } %"32_022", { {} }* %"32_0", align 1
  store i16 %"30_023", i16* %"30_0", align 2
  %"32_024" = load { {} }, { {} }* %"32_0", align 1
  %"30_025" = load i16, i16* %"30_0", align 2
  switch i32 0, label %28 [
  ]

28:                                               ; preds = %cond_exit_18
  %29 = extractvalue { {} } %"32_024", 0
  store i16 %"30_025", i16* %"03", align 2
  br label %24
}

declare i32 @printf(i8*, ...)

declare void @abort()

declare void @__quantum__qis__h__body(i16)

declare void @__quantum__qis__rz__body(double, i16)

define void @_hl.main.4() {
alloca_block:
  %"53_0" = alloca { {} }, align 8
  %"52_0" = alloca { {} }, align 8
  %"46_0" = alloca { {} }, align 8
  %"43_0" = alloca double, align 8
  %"44_0" = alloca { { double } }, align 8
  %"41_0" = alloca i16, align 2
  %"45_0" = alloca i16, align 2
  %"50_0" = alloca { i32, {}, {} }, align 8
  br label %entry_block

entry_block:                                      ; preds = %alloca_block
  br label %0

0:                                                ; preds = %entry_block
  store { {} } undef, { {} }* %"53_0", align 1
  %"53_01" = load { {} }, { {} }* %"53_0", align 1
  store { {} } %"53_01", { {} }* %"53_0", align 1
  store { {} } undef, { {} }* %"52_0", align 1
  store { {} } undef, { {} }* %"46_0", align 1
  store double 1.500000e+00, double* %"43_0", align 8
  %"43_02" = load double, double* %"43_0", align 8
  %1 = insertvalue { double } undef, double %"43_02", 0
  %2 = insertvalue { { double } } poison, { double } %1, 0
  store { { double } } %2, { { double } }* %"44_0", align 8
  %3 = call i16 @_hl.__new__.38()
  store i16 %3, i16* %"41_0", align 2
  %"41_03" = load i16, i16* %"41_0", align 2
  %"44_04" = load { { double } }, { { double } }* %"44_0", align 8
  %4 = call i16 @_hl.rx.1(i16 %"41_03", { { double } } %"44_04")
  store i16 %4, i16* %"45_0", align 2
  %"45_05" = load i16, i16* %"45_0", align 2
  %5 = call { i32, {}, {} } @_hl.measure.47(i16 %"45_05")
  store { i32, {}, {} } %5, { i32, {}, {} }* %"50_0", align 4
  %"50_06" = load { i32, {}, {} }, { i32, {}, {} }* %"50_0", align 4
  %6 = extractvalue { i32, {}, {} } %"50_06", 0
  %7 = trunc i32 %6 to i1
  call void @__quantum__rt__bool_record_output(i1 %7, i8* getelementptr inbounds ([2 x i8], [2 x i8]* @1, i32 0, i32 0))
  %"53_07" = load { {} }, { {} }* %"53_0", align 1
  switch i32 0, label %8 [
  ]

8:                                                ; preds = %0
  %9 = extractvalue { {} } %"53_07", 0
  br label %10

10:                                               ; preds = %8
  ret void
}

define i16 @_hl.__new__.38() {
alloca_block:
  %"0" = alloca i16, align 2
  %"54_0" = alloca i16, align 2
  %"01" = alloca i16, align 2
  %"62_0" = alloca { {} }, align 8
  %"60_0" = alloca i16, align 2
  %"61_0" = alloca { {} }, align 8
  %"59_0" = alloca i16, align 2
  br label %entry_block

entry_block:                                      ; preds = %alloca_block
  br label %0

0:                                                ; preds = %entry_block
  store { {} } undef, { {} }* %"62_0", align 1
  store { {} } undef, { {} }* %"61_0", align 1
  %1 = call i16 @__quantum__rt__qubit_allocate()
  store i16 %1, i16* %"59_0", align 2
  %"59_02" = load i16, i16* %"59_0", align 2
  call void @__quantum__qis__reset__body(i16 %"59_02")
  store i16 %"59_02", i16* %"60_0", align 2
  %"62_03" = load { {} }, { {} }* %"62_0", align 1
  %"60_04" = load i16, i16* %"60_0", align 2
  store { {} } %"62_03", { {} }* %"62_0", align 1
  store i16 %"60_04", i16* %"60_0", align 2
  %"62_05" = load { {} }, { {} }* %"62_0", align 1
  %"60_06" = load i16, i16* %"60_0", align 2
  switch i32 0, label %2 [
  ]

2:                                                ; preds = %0
  %3 = extractvalue { {} } %"62_05", 0
  store i16 %"60_06", i16* %"01", align 2
  br label %4

4:                                                ; preds = %2
  %"07" = load i16, i16* %"01", align 2
  store i16 %"07", i16* %"54_0", align 2
  %"54_08" = load i16, i16* %"54_0", align 2
  store i16 %"54_08", i16* %"0", align 2
  %"09" = load i16, i16* %"0", align 2
  ret i16 %"09"
}

define { i32, {}, {} } @_hl.measure.47(i16 %0) {
alloca_block:
  %"0" = alloca { i32, {}, {} }, align 8
  %"48_0" = alloca i16, align 2
  %"63_0" = alloca { i32, {}, {} }, align 8
  %"65_0" = alloca i16, align 2
  %"02" = alloca { i32, {}, {} }, align 8
  %"71_0" = alloca { {} }, align 8
  %"68_1" = alloca { i32, {}, {} }, align 8
  %"70_0" = alloca { {} }, align 8
  %"68_0" = alloca i16, align 2
  br label %entry_block

entry_block:                                      ; preds = %alloca_block
  store i16 %0, i16* %"48_0", align 2
  %"48_01" = load i16, i16* %"48_0", align 2
  store i16 %"48_01", i16* %"65_0", align 2
  br label %1

1:                                                ; preds = %entry_block
  %"65_03" = load i16, i16* %"65_0", align 2
  store { {} } undef, { {} }* %"71_0", align 1
  store { {} } undef, { {} }* %"70_0", align 1
  store i16 %"65_03", i16* %"65_0", align 2
  %"65_04" = load i16, i16* %"65_0", align 2
  %2 = call %RESULT* @__quantum__qis__m__body(i16 %"65_04")
  %3 = call i1 @__quantum__qis__read_result__body(%RESULT* %2)
  %4 = select i1 %3, { i32, {}, {} } { i32 1, {} poison, {} undef }, { i32, {}, {} } { i32 0, {} undef, {} poison }
  store i16 %"65_04", i16* %"68_0", align 2
  store { i32, {}, {} } %4, { i32, {}, {} }* %"68_1", align 4
  %"71_05" = load { {} }, { {} }* %"71_0", align 1
  %"68_16" = load { i32, {}, {} }, { i32, {}, {} }* %"68_1", align 4
  store { {} } %"71_05", { {} }* %"71_0", align 1
  store { i32, {}, {} } %"68_16", { i32, {}, {} }* %"68_1", align 4
  %"68_07" = load i16, i16* %"68_0", align 2
  call void @__quantum__rt__qubit_release(i16 %"68_07")
  %"71_08" = load { {} }, { {} }* %"71_0", align 1
  %"68_19" = load { i32, {}, {} }, { i32, {}, {} }* %"68_1", align 4
  switch i32 0, label %5 [
  ]

5:                                                ; preds = %1
  %6 = extractvalue { {} } %"71_08", 0
  store { i32, {}, {} } %"68_19", { i32, {}, {} }* %"02", align 4
  br label %7

7:                                                ; preds = %5
  %"010" = load { i32, {}, {} }, { i32, {}, {} }* %"02", align 4
  store { i32, {}, {} } %"010", { i32, {}, {} }* %"63_0", align 4
  %"63_011" = load { i32, {}, {} }, { i32, {}, {} }* %"63_0", align 4
  store { i32, {}, {} } %"63_011", { i32, {}, {} }* %"0", align 4
  %"012" = load { i32, {}, {} }, { i32, {}, {} }* %"0", align 4
  ret { i32, {}, {} } %"012"
}

declare void @__quantum__rt__bool_record_output(i1, i8*)

declare i16 @__quantum__rt__qubit_allocate()

declare void @__quantum__qis__reset__body(i16)

declare %RESULT* @__quantum__qis__m__body(i16)

declare i1 @__quantum__qis__read_result__body(%RESULT*)

declare void @__quantum__rt__qubit_release(i16)

export const meta = {
  name: 'night',
  description: "The night's Claude side, beside train/night.sh on the GPU: predict tonight's scores, grow the IQ suite (plan, teach, attack, solve), import it for tomorrow",
  whenToUse: 'From /night when compusophy goes to bed (ultracode on), or with {smoke: true} to try it by day on a scratch data root',
  phases: [
    { title: 'Predict', detail: "tonight's held-out rates, written before they are scored" },
    { title: 'Plan', detail: 'new families under new roots, by tier' },
    { title: 'Teach', detail: 'teachers write tasks and verify them' },
    { title: 'Attack', detail: 'an independent agent tries to break each task' },
    { title: 'Solve', detail: 'solvers write the training replies from the asks alone' },
    { title: 'Import', detail: "train/day.sh import and data: tomorrow night's inputs" },
  ],
}

// args (all optional but night, day, wave, repo): {night: 'n20261006', day: '2026-10-06',
// wave: 'w4', repo: '<worktree>', data: 'C:\\sept30\\computehub-data', tiers: [1..6],
// teachersPerTier: 2, familiesPerTeacher: 8, perFamily: 2, looseFrom: 3, deadline: none (only PAUSE stops it),
// importToo: true, smoke: false}
//
// A batch's tasks pass through the attacker before anything can import them: a teacher writes
// work/<key>/draft.jsonl, and only the attacker writes stage/<key>.jsonl (all at once, at its
// end), the file `day.sh import` takes. Each step skips what a run before it finished, so the
// same wave run again (after a freeze or a pause) resumes; Import writes work/<wave>/imported.json
// when the wave is in the suite.
const A = args || {}
const SMOKE = !!A.smoke
const REPO = A.repo || 'C:\\sept30\\computehub'
const DATA = A.data || 'C:\\sept30\\computehub-data'
const IQD = DATA + '\\iq'
const NIGHT = A.night || 'nsmoke'
const DAY = A.day || '2026-01-01'
const WAVE = A.wave || (SMOKE ? 'smoke' : 'w')
const TIERS = A.tiers || (SMOKE ? [1] : [1, 2, 3, 4, 5, 6])
const TEACHERS = A.teachersPerTier || (SMOKE ? 1 : 2)
const FAMILIES = A.familiesPerTeacher || (SMOKE ? 1 : 8)
const PER = A.perFamily || 2
// Loose asks (as people type them) from this tier up: below it, checks barred from labels and
// numbers rarely kill half the mutants `iq verify` asks for. 0: none.
const LOOSE_FROM = A.looseFrom === undefined ? 3 : A.looseFrom
// No clock: the night runs until compusophy says they are up, and the session touches PAUSE then.
// A deadline ('HH:MM') is only for a run that must end by a time.
const DEADLINE = A.deadline || ''
const IMPORT = A.importToo !== false && !SMOKE
const TEACH = REPO + '\\target\\release\\teach.exe'
const IQ = REPO + '\\target\\release\\iq.exe'
const SUITE = REPO + '\\evals\\suites\\iq.jsonl'

const RULES = `Ground rules (the night's; compusophy is asleep and the GPU is training):
- Never use the GPU: never run train/night.sh, sft.py, generate.py, ask.py, serve.py or export.py, nor anything that loads a model.
- Never edit the repository (${REPO}): read it only. The suite (${SUITE}) changes only through \`train/day.sh import\`, in the Import phase.
- In the data root (${DATA}) write only your own files, named below; never delete anything recursively (no rm -r/-rf, find -delete, Remove-Item -Recurse); never build with cargo (the release binaries are built): TEACH = ${TEACH}, IQ = ${IQ}.
- Make no AI or API calls of your own.
- Time and pause: before each task you start, ${DEADLINE ? `run \`date +%H:%M\` and ` : ''}check whether ${IQD}\\PAUSE exists. If ${DEADLINE ? `it is past ${DEADLINE} (and before 18:00), or ` : ''}PAUSE exists, stop: leave every file you made valid (a draft or stage file only with tasks that verify, a replies file only with lines that pass), and return what you finished, saying why you stopped.
- Shell: Git Bash; give tools Windows paths with forward slashes or quoted backslashes; write JSON lines with Python's json.dumps, never by hand.`

const LESSONS = `What earlier teachers learned (also in applang's card, which the writer prompt shows): reserved words (row, col, line, ring, rect, circle, text, clear, ...) never name states, functions or parameters; strings are read whole (letters as lists of one-letter strings); an icon's ring takes x y r; states start as literals and nothing runs on open (setup goes in Start's handler); a drag runs a canvas handler for every unit crossed (steer or paint on a canvas, never toggle); where chance or timing decides, the ask fixes it; one base timer is fairer to check than several. The held-out split goes by a family's ROOT (its name before the first '-'): keep every family name exactly as planned. Icon checks of references are replies files kept INSIDE your work folder, never ${IQD}\\replies-*.jsonl (those are the solvers' training data).`

const batches = []
for (const t of TIERS) for (let b = 1; b <= TEACHERS; b++) batches.push({ tier: t, b, key: `${WAVE}-t${t}-${b}` })
const stageOf = (x) => `${IQD}\\stage\\${x.key}.jsonl`
const workOf = (x) => `${IQD}\\work\\${x.key}`
const draftOf = (x) => `${workOf(x)}\\draft.jsonl`
const looseAt = (t) => LOOSE_FROM > 0 && t >= LOOSE_FROM && PER > 1
const repliesOf = (x) => `${IQD}\\replies-${x.key}.jsonl`

// ---------- Predict: before tonight is scored ----------
const PREDICT = { type: 'object', properties: { file: { type: 'string' }, roles: { type: 'array', items: { type: 'object', properties: { role: { type: 'string' }, held: { type: 'number' }, lo: { type: 'number' }, hi: { type: 'number' } }, required: ['role', 'held', 'lo', 'hi'] } }, notes: { type: 'string' } }, required: ['file', 'roles', 'notes'] }

const predictPrompt = `You predict tonight's held-out scores for compusophyOS's applang model, BEFORE they exist: DESIGN.md (${REPO}\\DESIGN.md), "## Evolution", "Prediction first", says why (a prediction written before the outcome; calibration is measured). Night ${NIGHT}.
${RULES}
Read: ${IQD}\\iq-history.jsonl and the report-*.md files (past nights), ${IQD}\\night-${NIGHT}\\ (tonight's copied inputs: count the records of sft.jsonl, the held-out prompts, held.txt) if it exists (else the same files in ${IQD}), train/night.sh (which roles run tonight, at which samples a task: base-q05 and base-q3 baselines once ever, q05 full fine-tune, q3 LoRA, q05-self), and ${REPO}\\DESIGN.md's "The applang model" numbers. Do NOT read any answers file of tonight (answers-${NIGHT}-*.jsonl), any report or history line for ${NIGHT}, nor the night log past its first lines: the point is to predict before seeing.
For each role tonight will score on the held-out tasks (q05, q3, q05-self; base-q05 and base-q3 if their baselines are not complete; never glm, already scored), predict its held-out pass rate as a fraction, with an 80% interval (lo, hi) honest about the noise: about 67 tasks in 34 families, one standard error of 5 to 6 points at 34%, less near 0; samples within a task correlate. Reason from the data (night 1: on its split the 0.5B fine-tune passed 4 of 100, all on one family since moved to train; the 3B LoRA 9 of 48 on 12 easy tasks; today's data is 428 records against night 1's 150), and say why in a sentence each.
If ${IQD}\\predictions-${NIGHT}.jsonl already exists, do not change it: return what it holds. Else write it, one JSON line a role: {"role","held","lo","hi","why","made":"<date +%F %H:%M>","by":"claude-code/claude-opus-5-5"}. Be quick: tonight's first scores land within the hour.`

// ---------- Plan: families under new roots ----------
const PLAN = { type: 'object', properties: { file: { type: 'string' }, batches: { type: 'array', items: { type: 'object', properties: { key: { type: 'string' }, tier: { type: 'number' }, families: { type: 'array', items: { type: 'string' } } }, required: ['key', 'tier', 'families'] } }, notes: { type: 'string' } }, required: ['file', 'batches', 'notes'] }

const planPrompt = `You plan tonight's wave (${WAVE}) of new benchmark families for compusophyOS's applang model: ${batches.length} teachers, ${FAMILIES} families each, for tiers ${TIERS.join(', ')} (${TEACHERS} teacher${TEACHERS > 1 ? 's' : ''} a tier). Their keys: ${batches.map(x => x.key).join(', ')}.
${RULES}
If ${IQD}\\work\\${WAVE}\\plan.json exists, a run of this wave began before (a freeze, a resume): return it unchanged. Else:
1. Read the tier ladder in \`${TEACH} writer --tier 1 --families x --per 1\` (and the other tiers' descriptions there), and every family already in use or promised: the suite (${SUITE}, the "family" of each line), every ${IQD}\\stage\\*.jsonl, and every other wave's plan, ${IQD}\\work\\*\\plan.json (a wave may be running beside this one). Compute every ROOT in use (a family's name before its first '-').
2. Choose ${FAMILIES} families for each teacher. Each family's ROOT is new: no family in the suite or the stage files has it, and no two planned families share one. Never a twin, under a new root, of an existing family OR of another family in this plan (a "focus-timer" when "pomodoro" exists, a tier-1 and a tier-3 version of one everyday app): twins would leak across the held-out split, which goes by root. Lowercase a-z0-9 and '-', short.
3. What to choose: the mission is personal software for everyone (DESIGN.md, Evolution, "What is selected for"): everyday apps one person or family needs (chores, budgets, logs of health, study, a small shop or club, a hobby, a household), and at the higher tiers the games and simulations people love; varied in what they exercise (lists, timers, boards, canvases, text, numbers, keys, drags). Fit each tier as the ladder says: tier 1 the simplest, tier 6 an ambitious game.
4. Write ${IQD}\\work\\${WAVE}\\plan.json: {"wave":"${WAVE}","batches":[{"key","tier","families":[...]}...]} and return it.`

// ---------- Teach ----------
const TAUGHT = { type: 'object', properties: { stage: { type: 'string' }, kept: { type: 'number' }, refused: { type: 'number' }, loose: { type: 'number' }, stopped: { type: 'string' }, notes: { type: 'string' } }, required: ['stage', 'kept', 'notes'] }

const teachPrompt = (x, fams) => `You are a teacher for compusophyOS's applang model (wave ${WAVE}, tier ${x.tier}, batch ${x.key}). You write benchmark tasks: an app request (the ask), a checker script and a reference program in applang. They become evaluation and training data for a small model fine-tuned nightly, so quality and honesty matter more than speed.
${RULES}
Your families: ${fams.join(', ')} (${PER} task${PER > 1 ? 's' : ''} each, ${fams.length * PER} in all). Your files: the draft ${draftOf(x)} and the rest of your work folder ${workOf(x)}\\ (never ${workOf(x)}\\attack\\). The stage file ${stageOf(x)} is the attacker's to write, never yours.
If ${stageOf(x)} exists, or ${workOf(x)}\\attack\\ does, the attacker has the batch: change nothing, and return the draft's count. Else, if the draft exists, a run began before: verify it (\`${IQ} verify <draft>\`), finish what is missing, and return.
1. Read the exact writer prompt: \`${TEACH} writer --tier ${x.tier} --families ${fams.join(',')} --per ${PER}\` (Studio's system prompt with applang's card and examples, the checker language, the tier ladder, the task format), and a few tier ${x.tier} tasks of the suite (${SUITE}) for the standard. Never reuse an id.
2. Write each task from separate ask, check and ref files in your work folder; build the draft with Python json.dumps, one object a line: {"id","tier","family","ask","check","ref","by":{"teacher":"claude-code/claude-opus-5-5","prompt":"","verifier":"<16 hex: the verifier \`${IQ} verify\` prints>","day":"${DAY}"}} (ids lowercase a-z0-9-, beginning with the family and a dash).
3. Verify until every task is kept: \`${IQ} verify <draft> --survivors\`. Check every reference's icon draws: a replies file of the refs in your work folder ({"task":id,"reply":"\`\`\`app\\n"+ref+"\\n\`\`\`"} a line) through \`${TEACH} replies --suite <draft> --replies <that file> --held <an empty file> --teacher check --out <a scratch file in your work folder>\` reports all passed. Fairness: grade 2 or 3 deliberately different correct programs per family with \`${IQ} grade <id> <file> --suite <draft>\` (all pass) and a deliberately wrong reading (fails).
${looseAt(x.tier) ? `4. Loose asks: in each family, the LAST task's ask is written the way a person types it: short and loose ("a chart of my kids' chores they can tick off"), with no hidden spec. Its check tests only what any reasonable app meeting that ask must do (the behavior asked for), never a label, layout or number the ask leaves open: where the app must be driven by a button the ask does not name, name the likely words as alternatives ("add|+|save", or press ... "enter"); keep the reference lean, one reasonable reading, without decorative labels. The other tasks keep precise asks that state the observable behavior the check relies on.` : ''}
${LESSONS}
Standards: the ask is what a person would type plus exactly the observable behavior the check relies on; the check tests that and nothing incidental; the reference is clean and idiomatic (under about 200 lines), with the header comment, the icon line and the label naming the app; the tasks of a family differ in what they ask; tier ${x.tier} as the ladder says. If a family cannot be made fair, leave it out and say so.`

// ---------- Attack ----------
const ATTACKED = { type: 'object', properties: { stage: { type: 'string' }, kept: { type: 'number' }, fixed: { type: 'number' }, dropped: { type: 'number' }, findings: { type: 'string' } }, required: ['stage', 'kept', 'fixed', 'dropped', 'findings'] }

const attackPrompt = (x) => `You attack benchmark tasks for compusophyOS's applang model, as an independent reviewer, so that only fair tasks with teeth survive: batch ${x.key}, the draft ${draftOf(x)} (another agent wrote it). Nothing reaches the suite but what you pass on.
${RULES}
Your files: ${workOf(x)}\\attack\\ (your own folder: copy the draft there as tasks.jsonl and work on that copy; never edit the teacher's files), and, at your very end, the stage file ${stageOf(x)}.
If ${workOf(x)}\\attack\\done.json exists, you finished before: return what it holds. If your folder exists without it, a run began before: go on from your copy.
For each task: first read only its ask (and, once, the writer prompt, \`${TEACH} writer --tier ${x.tier} --families x --per 1\`, for applang's card and the reply format). From the ask alone:
1. Write 2 correct programs that differ from each other in structure, labels and layout wherever the ask leaves them open, and grade each: \`${IQ} grade <id> <file> --suite <your copy>\`. A correct program that fails means the check tests what the ask does not say: fix the check (test less) or a precise ask (state the behavior), or drop the task. A loose ask (short, the way a person types) is never made precise: fix its check, or drop it.
2. Write 2 plausible wrong programs (a misreading of the ask, an off-by-one, an edge the ask states but the program ignores, a button that does nothing) and grade them. A wrong program that passes means the check lacks teeth: strengthen it, or drop the task.
Then read the check and the reference, and judge: does the check test what the ask asks, and nothing incidental? Is a loose ask graded only on what any reasonable app must do?
After any change: \`${IQ} verify <your copy>\` must keep every task left (the reference passes, a null app fails, the mutants mostly die), and the reference's icon must still draw (\`${TEACH} replies\` on the refs, as the teacher checked it, in your folder).
Last, and only when every task is judged: if any task is left, write them to ${stageOf(x)}.part, then \`mv\` it to ${stageOf(x)} (the file \`day.sh import\` takes); write ${workOf(x)}\\attack\\done.json ({"kept","fixed","dropped","findings"}); return it. If you stop early (time, PAUSE), write no stage file: the batch waits for a run that finishes it.`

// ---------- Solve ----------
const SOLVED = { type: 'object', properties: { replies: { type: 'string' }, graded: { type: 'number' }, passed: { type: 'number' }, unsolved: { type: 'string' }, misleading: { type: 'string' } }, required: ['replies', 'graded', 'passed', 'misleading'] }

const solvePrompt = (x) => `You are the teacher for compusophyOS's applang model, in the solver role, for the tasks staged in ${stageOf(x)} (batch ${x.key}). You write the replies a small model will be trained to give, in the exact format Studio uses.
${RULES}
Your files: ${repliesOf(x)} and your folder ${IQD}\\work\\solve-${x.key}\\.
1. Make the exact prompts: \`${TEACH} prompts --suite ${stageOf(x)} --split train --out ${IQD}\\work\\solve-${x.key}\\prompts.jsonl\` (held-out families are left out on purpose: never solve one).
2. For each task: read its system prompt once, fully (it defines applang and how a reply must look: the fenced \`\`\`app block, the header comment, the icon line, the label naming the app), and its ask; write the reply from the ask alone. Never read the task's "ref" or "check", nor anything in ${workOf(x)}\\ (the teacher's and the attacker's programs).
3. Test each program: \`${IQ} grade <task-id> <file> --suite ${stageOf(x)}\`; fix it as a careful author would until it passes.
4. Append each final reply as one line to ${repliesOf(x)}: {"task":"<id>","reply":"<the whole reply>"} via json.dumps; if the file exists, skip the tasks it already holds.
5. Confirm: \`${TEACH} replies --suite ${stageOf(x)} --replies ${repliesOf(x)} --teacher check --out <a scratch file in your folder>\` prints N graded, N passed. Say which asks misled you (a check that failed a fair reading): the Attack phase missed those.`

// ---------- Import ----------
const IMPORTED = { type: 'object', properties: { suite_tasks: { type: 'number' }, train: { type: 'number' }, held: { type: 'number' }, records: { type: 'number' }, refused: { type: 'string' }, glm: { type: 'string' }, notes: { type: 'string' } }, required: ['suite_tasks', 'train', 'held', 'records', 'notes'] }

const importPrompt = `You make tomorrow night's inputs from tonight's wave (${WAVE}) of compusophyOS's applang data.
${RULES}
Here, and only here, you run the day pipeline, which writes the suite: from ${REPO} (Git Bash), \`bash train/day.sh import\` (each stage file not yet imported into the suite, verified, then the prompts and held-out list made again, and GLM 5.3 asked the new held-out tasks through the free endpoint, paced; tonight's GPU run reads only the inputs it copied at its start, GLM's answers among them, so this is safe), then \`bash train/day.sh data\` (the solvers' replies and the references graded and exported as ${IQD}\\sft.jsonl). Neither the time limit nor PAUSE applies to you (day.sh uses no GPU): run both to the end. Never edit the suite by hand. When both are done, write ${IQD}\\work\\${WAVE}\\imported.json ({"suite_tasks","train","held","records","day":"<date +%F %H:%M>"}). Report from ${IQD}\\day.log: the suite's size, train and held-out tasks, sft.jsonl's records, any refused task (the *.refused.jsonl files beside the stage files) and why, and what GLM answered.`

// ---------- Run ----------
phase('Predict')
const predicted = SMOKE ? null : agent(predictPrompt, { label: 'predict', phase: 'Predict', schema: PREDICT })

phase('Plan')
const plan = await agent(planPrompt, { label: `plan:${WAVE}`, phase: 'Plan', schema: PLAN })
if (!plan || !plan.batches) throw new Error('no plan')
const famsOf = (x) => ((plan.batches.find(p => p.key === x.key) || {}).families || []).slice(0, FAMILIES)
log(`plan: ${plan.batches.map(p => p.key + ' ' + (p.families || []).length).join(', ')}`)

const results = await pipeline(batches,
  (x) => famsOf(x).length
    ? agent(teachPrompt(x, famsOf(x)), { label: `teach:${x.key}`, phase: 'Teach', schema: TAUGHT })
    : null,
  (taught, x) => taught && taught.kept > 0
    ? agent(attackPrompt(x), { label: `attack:${x.key}`, phase: 'Attack', schema: ATTACKED }).then(a => ({ taught, a }))
    : ({ taught, a: null }),
  // Only an attacked batch is solved: its stage file exists only then. Attackers count "kept"
  // two ways (left unchanged, or left in all), so a batch with any task kept or fixed is solved.
  (r, x) => r && r.taught && r.a && (r.a.kept || 0) + (r.a.fixed || 0) > 0
    ? agent(solvePrompt(x), { label: `solve:${x.key}`, phase: 'Solve', schema: SOLVED }).then(s => ({ ...r, s }))
    : r,
)

const batchesOut = results.map((r, i) => ({
  key: batches[i].key,
  drafted: r && r.taught ? r.taught.kept : 0,
  attack: r && r.a ? { kept: r.a.kept, fixed: r.a.fixed, dropped: r.a.dropped } : 'unattacked: not staged',
  solved: r && r.s ? `${r.s.passed}/${r.s.graded}` : null,
  misleading: r && r.s ? r.s.misleading : '',
  stopped: r && r.taught ? r.taught.stopped || '' : 'no result',
}))
log(`staged: ${batchesOut.map(b => b.key + ' ' + (b.attack && b.attack.kept !== undefined ? b.attack.kept : 'none')).join(', ')}`)

let imported = null
if (IMPORT) {
  phase('Import')
  imported = await agent(importPrompt, { label: 'import', phase: 'Import', schema: IMPORTED })
}
return { wave: WAVE, night: NIGHT, predicted: predicted ? await predicted : null, plan: plan.batches, batches: batchesOut, imported }

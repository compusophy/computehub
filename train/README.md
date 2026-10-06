# train/: fine-tuning a small open model on applang

The training side of the night loop: `teach` (Claude Opus as the teacher) writes SFT records,
this folder fine-tunes a small open model on them on the local RTX 3090, exports it to GGUF and
serves it, and `iq` scores it. A build-time tool that never ships, like the wasm-bindgen CLI:
CLAUDE.md's "Rust only" governs what ships; this is Python because the training stack is.

Nothing heavy lives in the repo. Every output goes under the **data root**: `--root`, else
`$COMPUTEHUB_DATA`, else `C:\sept30\computehub-data` (on the SSD; it must be outside the repo).

```
<root>/
  runs/<run>/manifest.json    provenance: data hashes, base revision, settings, losses, exports
  runs/<run>/train.log        every line sft.py printed, every attempt
  runs/<run>/ckpt-a, ckpt-b/  resume points, written in turn
  runs/<run>/adapter/         the LoRA adapter (or model/ for --full)
  runs/<run>/model-q8_0.gguf  the export llama-server serves
  runs/<run>/score.txt        what SCORE_CMD printed (night.sh)
  base/<base>-<rev>-q8_0.gguf an untouched base, for baselines (+ .json)
  scratch/                    the merged model and f16 GGUFs, one per base, overwritten in place
  serve/llama-server-<port>.json  the server's PID and start time
  logs/                       night-<run>.log, llama-server-<port>.log, serve.log
  runs.jsonl                  one line a night: what trained, on what, how it scored
```

Needs: Python 3.9+ with torch (CUDA), transformers, peft, accelerate, safetensors, psutil;
the base models in the local Hugging Face cache (the scripts run offline and never download);
a llama.cpp checkout (`--llama`, `$LLAMA_CPP_DIR`, else `C:\llama-cpp`) with
`convert_hf_to_gguf.py` and built `llama-server` and `llama-quantize`. A CUDA build in
`build-cuda/` is used before `build/`.

## The scripts

| file | what it does |
|---|---|
| `sft.py` | LoRA (r32, alpha 64, dropout 0.05 on q/k/v/o/gate/up/down) or `--full` fine-tune; bf16; gradient checkpointing; no packing; checkpoints and resumes; writes adapter + manifest + log |
| `sftdata.py` | what sft.py trains on: reads and checks records, refuses held-out tasks, renders prompts, splits off the eval slice, plans batches, counts tokens |
| `export.py` | merges the adapter into its base, saves it as a Hugging Face model, converts it to GGUF, quantizes it (`q8_0` default, `q4_k_m`, `f16`, `bf16`); `--base-only` converts an untouched base |
| `serve.py` | starts `llama-server` on 127.0.0.1:8081 (ctx 8192 a slot, every layer on the GPU), waits for `/health`, checks the chat template, prints the URL; `--stop`, `--status` |
| `day.sh` | the day's data, around the teacher (Claude Code sessions): `import` the teachers' tasks into the suite (verified), the prompts and held-out list again, GLM on new held-out tasks; `data` the solvers' replies and the references graded and exported as `sft.jsonl` |
| `night.sh` | the night: baselines (once ever), fine-tunes from base (0.5B full, 3B LoRA), a self-taught round, all scored on the held-out tasks; `report.py` at the end |
| `generate.py` | answers `teach prompts` on the GPU in batches, by the model's own chat template; stops each sample where Studio stops reading; resumes |
| `report.py` | each answers file scored by `iq score`: `iq-history.jsonl` (a line a model a night) and `report-<night>.md`, tonight beside the nights before |
| `common.py` | the data root, atomic writes, hashes, provenance, the base model from the cache |
| `smoke.py` | a tiny smoke set from `programs/makes/refs/*.app`, under SMOKE placeholder prompts |

```sh
python train/sft.py --data D.jsonl --held H.txt [--tasks evals/suites/iq.jsonl] [--base ID] [--run NAME]
python train/export.py --run NAME [--quant q8_0]
python train/export.py --base-only --base Qwen/Qwen2.5-Coder-0.5B-Instruct
python train/serve.py --run NAME          # or --base-only, or --gguf FILE [--tokenizer DIR]
python train/serve.py --stop
bash train/day.sh import && bash train/day.sh data   # by day, after the teachers and solvers
bash train/night.sh [--no-self] [--no-3b]            # from 22:00 to 08:00, or --force
```

Bases in the cache now: `Qwen/Qwen2.5-Coder-0.5B-Instruct` (the default),
`Qwen/Qwen2.5-Coder-3B-Instruct` (LoRA only: a full fine-tune does not fit 24 GB, so `--full`
refuses models over 1.6B parameters) and `Qwen/Qwen3-0.6B`.

## The SFT records

One JSON object a line, as `teach export` writes them:

```json
{"messages": [{"role": "system", "content": "..."}, {"role": "user", "content": "..."},
              {"role": "assistant", "content": "..."}],
 "task": "counter", "kind": "write",
 "by": {"teacher": "claude-opus-5-5", "prompt": "<16 hex>", "verifier": "<16 hex>", "day": "YYYY-MM-DD"}}
```

- `messages`: roles `system`, `user`, `assistant`; at least one user message; the last is the
  assistant's answer. `fix` records have more turns (the draft, the error, the fix); the loss
  falls on the **last assistant message only**.
- `task`: the task id, used to refuse held-out tasks and to pick the eval slice. A record may
  also carry `family`.
- `kind`, `by`: counted in the manifest (kinds, teachers, days), not trained on.

A record that breaks the schema stops the run, naming its file and line. Exact duplicates are
dropped and counted. A record over `--max-len` tokens (12288: Studio's system prompt alone is about 4,000) is dropped and counted, never cut.

**Training prompt = inference prompt.** Each record is rendered by the base model's own chat
template (`apply_chat_template`): the prompt is every message but the last plus the generation
prompt, which is what the server feeds the model; the answer is the rest of the full rendering
through the end-of-turn token. If the full rendering does not begin with the prompt's tokens,
the run stops. `serve.py` then asks the server to render and tokenize a chat
(`/apply-template`, `/tokenize`; the server runs the template the GGUF carries, with `--jinja`)
and stops if either differs from the tokenizer's. The system prompt is the record's own; the
manifest lists each distinct system prompt's hash, so a teacher prompt that drifts from the one
`iq` sends shows. For Qwen3 the answer includes the empty `<think></think>` block its template
writes.

## The held-out set

One of these is required:

- `--held FILE`: one task family per line (`#` comments). Families come from the suite,
  `--tasks` (default `evals/suites/iq.jsonl`): JSON lines with `id` and `family`; lines
  without an `id` (a header) are skipped. Every task of a held family is held, and of every
  family of its root (its name before the first `-`: `pong` holds `pong-ai`; or the root
  `JOINED` holds it with, as `iq::JOINED` does: `level-editor` falls with `platformer`), as
  `iq::held` splits them.
- `--held-tasks FILE|a,b,c`: task ids, for when there is no suite.
- `--no-held`: smoke tests only; the manifest says so.

Records of held tasks are refused (counted by family in the manifest, warned in the log), so
`iq` can score on tasks the model never saw. The eval loss is measured on a separate slice of
the training tasks (`--val-frac` 0.05 of the tasks, at most `--val-max` 64 records), picked
by a hash of the task id: the same slice every night, and never trained on.

## Provenance and crash safety

`manifest.json` holds: the SHA-256 and record count of each data file; the held-out files'
hashes and families; record counts (read, held, duplicate, too long, train, val); kinds,
teachers, days and system prompts; token lengths (prompt, answer, total: min, mean, p50, p90,
p99, max); the base id and revision (the cache's snapshot hash); every setting; the repo's git
commit (and whether it was dirty) and sft.py's hash; each attempt's start, end, PID, commit
and resume point; and at the end the train and eval losses (with their histories), steps,
seconds, tokens a second, peak VRAM, the output files' hashes, and each export's size, hash
and llama.cpp build.

Every round starts from the base model: a folder with an adapter or a fine-tune's
`computehub-run.json` (written into every adapter, model and merged folder), or one under the
data root, is refused as `--base`.

The machine can hard-freeze under long loads, so: every `--save-every` steps (50) sft.py writes
a checkpoint (trainable weights, optimizer, scheduler, RNG states, counters) into whichever of
`ckpt-a`/`ckpt-b` is older: it marks the slot invalid, writes each file to `.tmp`, flushes it
to the disk and renames it, and writes `state.json` last. A crash at any point leaves the
other slot whole. Rerunning the same command resumes from the newest whole slot whose settings
and data match (the batch order is a function of the seed and epoch, so the resumed run sees
what the first would have); other settings under the same `--run` name are refused. JSON files
are written the same way. Nothing is ever deleted: checkpoints, merged models and
intermediates are overwritten in place.

## The night run

`night.sh` reads `<root>/iq/`: `sft.jsonl`, `solutions.jsonl`, `prompts-held.jsonl`,
`prompts-train.jsonl` and `held.txt`, all made by day (`day.sh`). It refuses to start from
08:00 to 22:00 local (the owner games on this GPU by day) unless `--force`, or with under 50 GB
free. Then, each step skipped when its output exists:

1. Baselines, answered once ever: the untuned Qwen2.5-Coder 0.5B and 3B on the held-out tasks
   (`answers-base-q05.jsonl`, `answers-base-q3.jsonl`, k 2; new held-out tasks are answered as
   they come, since `generate.py` skips what is answered).
2. Tonight's fine-tunes, each from its base: 0.5B in full, 3B by LoRA, each scored on the
   held-out tasks (k 4).
3. A self-taught round: tonight's 0.5B answers the train tasks 8 times; its answers that pass
   (iq's check, and an icon that draws) join the teacher's, at most 2 a task, and a 0.5B is
   trained on both from its base and scored. Skipped with `--no-self`, or when it is past
   `SELF_BY` (4) o'clock.
4. `report.py`: `iq-history.jsonl` and `report-<night>.md`.

From 07:45 no new step starts and the report is written from what tonight has; the rest
resumes the next night. The night is named `n<evening's date>`, so a rerun after a freeze
resumes it (`sft.py` from its checkpoint, `generate.py` from its last batch). Its log is
`<root>/iq/night-<night>.log`.

## The smoke test

```sh
python train/smoke.py      # <root>/smoke/: sft.jsonl, tasks.jsonl, held.txt
python train/sft.py --data <root>/smoke/sft.jsonl --tasks <root>/smoke/tasks.jsonl \
    --held <root>/smoke/held.txt --run smoke --max-steps 30 --save-every 10
python train/export.py --run smoke
python train/serve.py --run smoke
curl -s http://127.0.0.1:8081/v1/chat/completions -H "Content-Type: application/json" \
    -d '{"temperature":0,"messages":[{"role":"user","content":"make a counter app"}]}'
python train/serve.py --stop
```

Measured on 2026-10-05 (28 records, 3 refused as held-out, 22 trained, 3 eval; prompt plus
answer 108 to 1,214 tokens): 0.5B LoRA, 30 steps of 16 sequences: 0.74 steps/s, ~3,650
tokens/s, 9.5 GB peak VRAM reserved (4.8 GB allocated), 40 s; export 24 s; q8_0 GGUF 506 MB
(q4_k_m 379 MB). 3B LoRA: ~1,300 tokens/s, 15 GB reserved. A run killed at step 15 and rerun
resumed from its step-10 checkpoint and finished with the uninterrupted run's losses (to bf16
noise).

## Fixed since tempo-x402's scripts

- **Prompt mismatch.** `finetune.py` hard-coded a ChatML string with its own system prompt;
  here the record's own messages go through the tokenizer's own template, and the server's
  rendering is checked against it.
- **Loss mask.** It masked by the character length of a re-tokenized prefix, which can slip a
  token at the boundary; here the prompt is a checked token prefix, and the loss covers the last
  assistant message through its end-of-turn token, nothing after.
- **Train = test.** Solutions to the 201 benchmark problems were trained on and scored on the
  same 201 problems. Here a held-out set must be named and its tasks are refused; the eval loss
  is on tasks never trained on.
- **Silent truncation** (`truncation=True`) and padding every row to max length: here over-long
  records are dropped and counted, and rows pad only to their length-sorted micro-batch.
- **fp16 LoRA** (overflow-prone): bf16 autocast, fp32 adapters; full fine-tunes keep fp32
  weights. The loss projects the vocabulary only at answer positions, in checkpointed chunks, so
  4,096-token records fit.
- **No provenance** beyond a few settings: the manifest above.
- **Checkpoints once an epoch, no resume**: alternating atomic slots every 50 steps, automatic
  and exact resume.
- **Dedup by problem, keeping the latest** solution: only exact duplicates are dropped;
  which records exist is `teach`'s call.
- **Export** to a temp folder removed afterwards, and q4_k_m for the benchmark: q8_0 by default
  for evals, fixed scratch folders overwritten in place, `.tmp` then rename, hashes recorded.
- **Serving** with `taskkill /F /IM llama-server.exe` (every llama-server on the machine) and
  one health probe after `sleep 5`: here a PID file with the start time, only that process
  stopped, `/health` polled, the server's log kept.
- Kept: every round restarts from the base model; now enforced.

## Open issues

- The llama.cpp build in `C:\llama-cpp\build` has no GPU backend (`GGML_CUDA=OFF`), so
  `llama-server` ignores `-ngl` and runs on the CPU (the 0.5B q8_0 answered at ~48 tokens/s).
  `serve.py` warns and records it. A CUDA build in `C:\llama-cpp\build-cuda` is picked up
  without changes (CUDA 11.7's `nvcc` is installed).
- Python 3.9 is past its end of life; transformers warns at every import.

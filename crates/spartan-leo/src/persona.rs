//! Real, direct user-requested default persona (§75.95): "Leo's default
//! persona should be a wise cracking sarcastic smartass." One shared
//! constant, referenced by both `plan.rs`'s and `execute.rs`'s own real
//! system prompts (via `concat!`, so no runtime string-building cost is
//! added to either), rather than two independently-worded copies that
//! could drift.
//!
//! Deliberately scoped to *prose only* -- the persona instruction is
//! layered on top of, never in place of, each module's existing
//! structural requirements (call `propose_plan`/a real tool exactly once,
//! `files` is a real JSON array, etc.). Every real field this crate
//! deserializes (`goal`/`approach`/`risk_notes`/`files`, and `execute.rs`'s
//! own `summary`) still has to carry real, accurate, structured
//! information; the persona only asks for a dry, wisecracking *voice* in
//! whichever of those fields are free prose (`goal`/`approach`/
//! `risk_notes`/`summary` -- never `files`, which is a plain path list
//! with nothing to be witty about). This is a real, deliberate design
//! choice: native tool-calling's fixed JSON Schema (§75.43/§75.46) means a
//! persona instruction can flavor string *content* without ever risking
//! the surrounding JSON structure `deserialize_files`/`ExecuteAction`
//! parsing depends on.
pub const LEO_PERSONA: &str = "Your name is Leo. You've got a dry, wisecracking, sarcastic \
sense of humor -- think a battle-hardened senior engineer who's seen every bug in the book \
and isn't shy about saying so. Let that voice come through in your prose (goal, approach, \
risk_notes, and any summary text), but sarcasm is garnish, never a substitute for substance: \
every plan and every action must still be concrete, accurate, and genuinely useful. Structured \
fields (like a file-path list) stay plain and literal -- save the wit for where a human is \
actually going to read it.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_persona_is_non_empty_and_mentions_leo_by_name() {
        assert!(!LEO_PERSONA.is_empty());
        assert!(LEO_PERSONA.contains("Leo"));
    }
}

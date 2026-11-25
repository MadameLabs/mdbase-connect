/** All question wording sent to Jev, in one place. Candidate and excerpt text is
 * data in `state`; it never appears here. Starting wording, to tune against the live API. */

export const CANDIDATE_INSTRUCTIONS = (id: string): string =>
  `Does \`candidates.${id}\` contain the answer to \`query\`, or is it the note a person asking \`query\` wants to open?`;

export const CANDIDATE_CRITERIA = {
  true: "the note answers the question or is the note being looked for",
  false: "the note only shares words with the question"
} as const;

export const PICK_INDEX_INSTRUCTIONS = "Which of these collection maps covers the subject of `query`?";

export const ANY_INDEX_INSTRUCTIONS = "Does any of these collection maps cover the subject of `query`?";

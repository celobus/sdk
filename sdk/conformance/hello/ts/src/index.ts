// The SDK conformance plugin `hello`, TypeScript. It declares the same roles, capability, dependency, enum,
// messages, entities and endpoints as the Rust and C# versions; `--describe` must print the same canonical
// descriptor (sdk/conformance/hello/descriptor.json).
import { endpoint, entity, plugin, role, t } from '@celobus/sdk';
import * as platform from '@celobus/sdk/platform';

// --- identity -------------------------------------------------------------------------------------

export const clerk = role({ title: 'Clerk' });
export const manager = role({ title: 'Manager', strict: true });

// --- data -----------------------------------------------------------------------------------------

export const mood = t.enum('Mood', ['happy', 'sad']);

export const helloRequest = t.message('HelloRequest', {
  name: t.text(),
  mood: mood.optional(),
});

export const helloReply = t.message('HelloReply', {
  text: t.text(),
  count: t.bigint(),
});

export const note = entity({
  key: 'i64',
  version: true,
  label: 'title',
  columns: {
    title: t.text().index().searchable(),
    body: t.text().optional().searchable('english'),
    ownerId: t.ref(platform.user),
    secret: t.text().optional(), // read by managers only (others read it as absent)
  },
  indexes: [['ownerId', 'createdAt desc'], { columns: ['ownerId', 'title'], unique: true }],
  // who reads and writes notes: the owner their own, whoever greeted on one by name that one, a manager every note
  // (and the secret)
  access: (a) => {
    const owner = a.relation('owner', a.column('ownerId'));
    const greeted = a.relation('greeted', a.rows(greeting, 'noteId', 'greeterId').when({ name: a.ne('') }));
    a.read(owner, greeted, manager);
    a.create(owner);
    a.update(owner);
    a.delete(owner, manager);
    a.mask('secret', [manager]);
  },
});

// A greeting: whom it greeted. The greetings are counted, not a counter kept.
export const greeting = entity({
  key: 'i64',
  appendOnly: true,
  columns: {
    name: t.text().index(),
    email: t.text().optional().encrypted(), // the greeted one's email, when known: kept encrypted at rest
    noteId: t.ref(() => note, { nav: 'note', inverse: 'greetings', onDelete: 'cascade' }).optional(), // the note it greeted on, if any
    greeterId: t.ref(platform.user, { onDelete: 'keep' }).optional(), // who greeted on it: they read the note
  },
  // who greets, and reads the greetings: whoever may call `Hello`
  access: (a) => {
    a.read(clerk, manager);
    a.create(clerk, manager);
  },
});

export const getNoteRequest = t.message('GetNoteRequest', {
  id: t.ref(() => note),
});

export const saveNoteRequest = t.message('SaveNoteRequest', {
  id: t.ref(() => note).optional(), // absent: a new note
  title: t.text(),
  body: t.text().optional(),
  ownerId: t.ref(platform.user),
});

export const deleteNoteRequest = t.message('DeleteNoteRequest', {
  id: t.ref(() => note),
});

export const listNotesRequest = t.message('ListNotesRequest', {
  limit: t.int(), // at most this many (0: 100)
});

export const listNotesReply = t.message('ListNotesReply', {
  titles: t.array(t.text()),
});

export const countAllNotesRequest = t.message('CountAllNotesRequest', {
  ownerId: t.ref(platform.user).optional(), // only this person's notes
});

export const noteCount = t.message('NoteCount', {
  count: t.bigint(),
});

export const notesByOwnerRequest = t.message('NotesByOwnerRequest', {
  minNotes: t.bigint(), // owners with at least this many notes
});

export const noteTally = t.message('NoteTally', {
  ownerId: t.ref(platform.user),
  notes: t.bigint(),
});

export const notesByOwnerReply = t.message('NotesByOwnerReply', {
  owners: t.array(noteTally),
});

// --- endpoints ------------------------------------------------------------------------------------

/** Greets, and counts the greetings of that name: a row each, in a transaction with its count. */
export const hello = endpoint({
  description: 'Greets.',
  input: helloRequest,
  output: helloReply,
  roles: [clerk, manager],
  async handler(ctx, { name, mood }) {
    const count = await ctx.transaction(async (tx) => {
      const greetings = tx.db(greeting);
      await greetings.create({ name });
      return greetings.count(greeting.f.name.eq(name));
    });
    const text = mood === 'sad' ? `Cheer up, ${name}.` : `Hello, ${name}!`;
    return { text, count: BigInt(count) };
  },
});

/** Reads a note (404 when there is none). */
export const getNote = endpoint({
  description: 'Reads a note.',
  input: getNoteRequest,
  output: note,
  readOnly: true,
  async handler(ctx, { id }) {
    return ctx.db(note).get(id);
  },
});

/** Creates a note, or changes one, in a transaction: the write, then a read of the owner's note with that title. */
export const saveNote = endpoint({
  description: 'Creates or changes a note.',
  input: saveNoteRequest,
  output: note,
  async handler(ctx, req) {
    return ctx.transaction(async (tx) => {
      const notes = tx.db(note);
      const saved = req.id
        ? await notes.update(req.id, { title: req.title, body: req.body ?? null })
        : await notes.create({ title: req.title, body: req.body, ownerId: req.ownerId });
      const mine = note.f.ownerId.eq(req.ownerId).and(note.f.title.eq(req.title));
      if (!(await notes.find(mine).orderBy(note.f.createdAt.desc()).first())) throw new Error('the saved note is not there');
      return saved;
    });
  },
});

/** Deletes a note and answers it as it was. */
export const deleteNote = endpoint({
  description: 'Deletes a note.',
  input: deleteNoteRequest,
  output: note,
  async handler(ctx, { id }) {
    const notes = ctx.db(note);
    const row = await notes.get(id);
    await notes.delete(id).expectVersion(row.version);
    return row;
  },
});

/** The titles of the notes the caller may read, newest first: their own, or every note for a manager. */
export const listNotes = endpoint({
  description: 'Lists the notes the caller may read.',
  input: listNotesRequest,
  output: listNotesReply,
  readOnly: true,
  async handler(ctx, { limit }) {
    const page = await ctx.db(note).find().orderBy(note.f.createdAt.desc()).limit(limit > 0 ? limit : 100).page();
    return { titles: page.rows.map((n) => n.title) };
  },
});

/** Counts every note of the workspace (or one person's), whoever may read them: a system context, audited. */
export const countAllNotes = endpoint({
  description: 'Counts every note, as the system.',
  input: countAllNotesRequest,
  output: noteCount,
  roles: [manager],
  readOnly: true,
  async handler(ctx, { ownerId }) {
    const notes = ctx.asSystem('count every note').db(note);
    const count = ownerId ? await notes.count(note.f.ownerId.eq(ownerId)) : await notes.count();
    return { count: BigInt(count) };
  },
});

/**
 * Counts the notes of each owner the caller may read: SQL over the notes' secure view (`hello_note`), run as the
 * caller through the notes' rules; its rows are JSON objects by column name.
 */
export const notesByOwner = endpoint({
  description: 'Counts the notes of each owner the caller may read, with SQL.',
  input: notesByOwnerRequest,
  output: notesByOwnerReply,
  readOnly: true,
  async handler(ctx, req) {
    const rows = await ctx.sql<{ owner_id: string; notes: number }>(
      'select owner_id, count(*) as notes from hello_note group by owner_id having count(*) >= $1 order by owner_id',
      req.minNotes,
    );
    return { owners: rows.map((r) => ({ ownerId: r.owner_id, notes: BigInt(r.notes) })) };
  },
});

export default plugin({
  id: 'hello', // the version is package.json's
  title: 'Hello',
  description: 'The SDK conformance plugin.',
  roles: { clerk, manager },
  capabilities: {
    'push:send': 'Greets people.',
    'authz:bypass': { reason: 'Counts every note.', on: [{ entity: note, ops: ['read'] }] },
  },
  dependencies: [{ plugin: 'greeter', calls: ['Greet'] }],
  entities: { note, greeting },
  endpoints: { hello, getNote, saveNote, deleteNote, listNotes, countAllNotes, notesByOwner },
});

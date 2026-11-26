// The SDK conformance plugin `prefs`: a plugin's settings, the same in Rust, TypeScript and C#. Its settings hold every
// type a setting takes (text, a number of each kind, a decimal, a date, a bool, an enum, lists of enums, texts and
// members, a member, a secret) and every validator, in sections. `--describe` prints the form derived from them and must
// equal `../descriptor.json`; with `--form` the plugin lays the form out by hand and checks each change (`onChange`),
// and `--describe` must equal `../descriptor-form.json` (`scripts/sdk-conformance.sh`).
import { Decimal, endpoint, invalid, plugin, settingsDef, t, type Plugin } from '@celobus/sdk';
import * as platform from '@celobus/sdk/platform';

export const tone = t.enum('Tone', ['calm', 'loud_and_clear']);
export const channel = t.enum('Channel', ['email', 'chat', 'push']);

export const prefsSettings = t.message(
  'PrefsSettings',
  {
    threadWarmHours: t.int()
      .describe('After this many hours without messages a thread no longer sticks to the agent that answered in it.')
      .label('Thread stays with its agent, hours').default(2).min(1).max(48).section('Agents'),
    messageMaxChars: t.bigint().describe('The longest message a member may send.')
      .default(20000n).min(100).max(20000).message('From 100 to 20,000 characters').section('Agents'),
    sampleRate: t.double().default(0.5).min(0).max(1),
    fee: t.decimal().default(Decimal.from('12.50')).min('0').max('100.00'),
    startsOn: t.date().optional().min('2020-01-01').max('2030-12-31'),
    enabled: t.bool().default(true),
    tone: t.enum(tone).default('calm'),
    channels: t.array(t.enum(channel)).label('Notify by').default(['email']).minItems(1).maxItems(2),
    tags: t.array(t.text()).maxItems(5).placeholder('One per line'),
    watchers: t.array(t.ref(platform.user)).maxItems(3),
    owner: t.ref(platform.user).optional(),
    greeting: t.text().describe('The first message of a thread.').multiline().pattern('^[A-Z]').minLen(1).maxLen(500),
    webhookUrl: t.text().optional().label('Webhook URL').url().placeholder('https://').section('Integrations'),
    contact: t.text().optional().email().section('Integrations'),
    webhookToken: t.secret().optional().minLen(8).maxLen(200).section('Integrations'),
  },
  { description: 'What the owners and admins of a workspace set for the plugin.', kind: 'settings' },
);

export const hours = endpoint({
  description: 'How long a thread stays with its agent.',
  input: {},
  output: { hours: t.int(), tone },
  async handler(ctx) {
    const s = ctx.settings(prefsSettings);
    return { hours: s.threadWarmHours, tone: s.tone };
  },
});

/** The plugin; `form`: the form laid out by hand, and each change checked. */
export function prefs(form: boolean): Plugin {
  const settings = form
    ? settingsDef(prefsSettings, {
        onChange(_ctx, change) {
          // a token needs the URL it is for
          if (change.current.webhookToken !== undefined && change.current.webhookUrl === undefined) {
            throw invalid('a token needs a URL').field('webhook_url', 'Set the URL the token is for');
          }
        },
        form: (f) => {
          const s = prefsSettings.f;
          f.section('Agents');
          f.number(s.threadWarmHours, 'Hours').help('How long a thread stays with its agent');
          f.number(s.messageMaxChars, 'Longest message');
          f.section('Behaviour');
          f.number(s.sampleRate, 'Sample rate');
          f.number(s.fee, 'Fee');
          f.date(s.startsOn, 'Starts on');
          f.checkbox(s.enabled, 'Enabled');
          f.radio(s.tone, 'Tone');
          f.multiSelect(s.channels, 'Channels');
          f.textarea(s.tags, 'Tags');
          f.member(s.watchers, 'Watchers');
          f.member(s.owner, 'Owner');
          f.textarea(s.greeting, 'Greeting').placeholder('Hello!');
          f.section('Integrations');
          f.text(s.webhookUrl, 'URL').required();
          f.text(s.contact, 'Contact');
          f.secret(s.webhookToken, 'Token');
        },
      })
    : settingsDef(prefsSettings);
  return plugin({ id: 'prefs', endpoints: { hours }, settings });
}

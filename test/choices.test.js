import assert from 'node:assert/strict';
import test from 'node:test';
import getChoices from '../lib/choices.js';

const createChangeTypes = () => [
  {
    handle: 'major',
    name: 'Major Change',
    description: 'incompatible API change',
  },
  {
    handle: 'minor',
    name: 'Minor Change',
    description: 'backwards-compatible functionality',
  },
  {
    handle: 'patch',
    name: 'Patch',
    description: 'backwards-compatible bug fix',
  },
];

test('offers every change type for an initial release', () => {
  const choices = getChoices(createChangeTypes(), [{ version: '0.0.1' }]);

  assert.deepEqual(
    choices.filter((choice) => choice.value).map((choice) => choice.value),
    ['major', 'minor', 'patch', 'ignore']
  );
});

test('offers only patch and ignore for a patch release', () => {
  const choices = getChoices(createChangeTypes(), [{ version: '0.0.2' }, { version: '0.0.1' }]);

  assert.deepEqual(
    choices.filter((choice) => choice.value).map((choice) => choice.value),
    ['patch', 'ignore']
  );
});

import js from '@eslint/js';

export default [
    {
        ignores: ['node_modules/**', 'schemas/gschemas.compiled', '*.shell-extension.zip'],
    },
    js.configs.recommended,
    {
        files: ['**/*.js'],
        languageOptions: {
            ecmaVersion: 2022,
            sourceType: 'module',
            globals: {
                console: 'readonly',
                // GNOME Shell injects this into the extension scope.
                global: 'readonly',
                TextDecoder: 'readonly',
                TextEncoder: 'readonly',
            },
        },
        rules: {
            indent: ['error', 4, {SwitchCase: 0}],
            'no-unused-vars': ['error', {
                argsIgnorePattern: '^_',
                caughtErrorsIgnorePattern: '^_',
            }],
            quotes: ['error', 'single', {avoidEscape: true}],
            semi: ['error', 'always'],
        },
    },
];

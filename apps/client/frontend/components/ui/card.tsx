/**
 * Administrator panel sections. Each card fills one administrator panel, whose section navigation
 * already names it, so cards render flat and keep their header only for assistive technology.
 */
import * as React from 'react';
import * as stylex from '@stylexjs/stylex';
import { colors, lineHeights } from '@/lib/theme.stylex';

type CardElementProps<Element extends 'div' | 'h2'> = Omit<
  React.ComponentProps<Element>,
  'className'
> & { xstyle?: stylex.StyleXStyles };

/**
 * Render a flat administrator panel section.
 *
 * @param props - Native container properties and style overrides.
 * @returns Card container.
 */
function Card({ xstyle, ...props }: CardElementProps<'div'>) {
  return <div data-slot="card" {...stylex.props(styles.card, xstyle)} {...props} />;
}

/**
 * Render the visually hidden title block of a card.
 *
 * @param props - Native container properties and style overrides.
 * @returns Screen-reader-only card header.
 */
function CardHeader({ xstyle, ...props }: CardElementProps<'div'>) {
  return <div data-slot="card-header" {...stylex.props(styles.cardHeader, xstyle)} {...props} />;
}

/**
 * Render a card title.
 *
 * @param props - Native heading properties and style overrides.
 * @returns Card title.
 */
function CardTitle({ xstyle, ...props }: CardElementProps<'h2'>) {
  return <h2 data-slot="card-title" {...stylex.props(styles.cardTitle, xstyle)} {...props} />;
}

/**
 * Render a card description.
 *
 * @param props - Native container properties and style overrides.
 * @returns Card description.
 */
function CardDescription({ xstyle, ...props }: CardElementProps<'div'>) {
  return (
    <div
      data-slot="card-description"
      {...stylex.props(styles.cardDescription, xstyle)}
      {...props}
    />
  );
}

/**
 * Render the card body.
 *
 * @param props - Native container properties and style overrides.
 * @returns Card content.
 */
function CardContent({ xstyle, ...props }: CardElementProps<'div'>) {
  return <div data-slot="card-content" {...stylex.props(xstyle)} {...props} />;
}

export { Card, CardHeader, CardTitle, CardDescription, CardContent };

const styles = stylex.create({
  card: {
    display: 'flex',
    flexDirection: 'column',
    color: colors.cardForeground,
  },
  cardHeader: {
    position: 'absolute',
    width: '1px',
    height: '1px',
    padding: 0,
    margin: '-1px',
    overflow: 'hidden',
    clipPath: 'inset(50%)',
    whiteSpace: 'nowrap',
    borderWidth: 0,
  },
  cardTitle: {
    lineHeight: 1.375,
    fontWeight: 700,
  },
  cardDescription: {
    fontSize: '0.875rem',
    lineHeight: lineHeights.textSm,
    color: colors.mutedForeground,
  },
});

'use client';

/**
 * Animate UI's animated lucide icons under their lucide-react names. An icon plays when the control
 * holding it is hovered or tapped; controls opt in through `useIconTrigger` and
 * `IconTriggerProvider`. Until then it renders as the identical still lucide-react icon.
 */

import {
  createContext,
  use,
  useEffect,
  useState,
  type AriaAttributes,
  type ComponentType,
  type CSSProperties,
  type PointerEvent,
  type ReactNode,
} from 'react';
import type { IconProps } from '@animate-ui/react/icons';
import {
  AlertCircle as StillAlertCircle,
  AlertTriangle as StillAlertTriangle,
  ArrowDown as StillArrowDown,
  ArrowLeft as StillArrowLeft,
  ArrowUp as StillArrowUp,
  Bell as StillBell,
  BellRing as StillBellRing,
  Bookmark as StillBookmark,
  BookmarkCheck as StillBookmarkCheck,
  Bot as StillBot,
  CalendarDays as StillCalendarDays,
  Check as StillCheck,
  CheckCircle2 as StillCheckCircle2,
  ChevronDown as StillChevronDown,
  ChevronRight as StillChevronRight,
  ChevronUp as StillChevronUp,
  Clock as StillClock,
  Clock3 as StillClock3,
  Copy as StillCopy,
  ExternalLink as StillExternalLink,
  Download as StillDownload,
  Eye as StillEye,
  EyeOff as StillEyeOff,
  FileText as StillFileText,
  Folder as StillFolder,
  Key as StillKey,
  KeyRound as StillKeyRound,
  Layers as StillLayers,
  LogOut as StillLogOut,
  Maximize as StillMaximize,
  Megaphone as StillMegaphone,
  Minus as StillMinus,
  Moon as StillMoon,
  MoreHorizontal as StillMoreHorizontal,
  Pencil as StillPencil,
  Plus as StillPlus,
  RefreshCw as StillRefreshCw,
  RotateCcw as StillRotateCcw,
  Save as StillSave,
  Search as StillSearch,
  Send as StillSend,
  Settings as StillSettings,
  ShieldCheck as StillShieldCheck,
  SlidersHorizontal as StillSlidersHorizontal,
  Star as StillStar,
  Trash2 as StillTrash2,
  Users as StillUsers,
  Wand2 as StillWand2,
  X as StillX,
  BookOpen as StillBookOpen,
  CircleQuestionMark as StillCircleQuestionMark,
  Gift as StillGift,
  Link as StillLinkIcon,
  SendHorizontal as StillSendHorizontal,
  Share2 as StillShare2,
  CircleX as StillCircleX,
  List as StillList,
  SlidersVertical as StillSlidersVertical,
  Tag as StillTag,
  Unlink2 as StillUnlink2,
  SunMoon as StillSunMoon,
  Sun as StillSun,
  type LucideIcon,
} from 'lucide-react';

import { ArrowDown as AnimatedArrowDown } from '@/components/animate-ui/icons/arrow-down';
import { ArrowLeft as AnimatedArrowLeft } from '@/components/animate-ui/icons/arrow-left';
import { ArrowUp as AnimatedArrowUp } from '@/components/animate-ui/icons/arrow-up';
import { Bell as AnimatedBell } from '@/components/animate-ui/icons/bell';
import { BellRing as AnimatedBellRing } from '@/components/animate-ui/icons/bell-ring';
import { BookOpen as AnimatedBookOpen } from '@/components/animate-ui/icons/book-open';
import { Bookmark as AnimatedBookmark } from '@/components/animate-ui/icons/bookmark';
import { BookmarkCheck as AnimatedBookmarkCheck } from '@/components/animate-ui/icons/bookmark-check';
import { Bot as AnimatedBot } from '@/components/animate-ui/icons/bot';
import { CalendarDays as AnimatedCalendarDays } from '@/components/animate-ui/icons/calendar-days';
import { Check as AnimatedCheck } from '@/components/animate-ui/icons/check';
import { ChevronDown as AnimatedChevronDown } from '@/components/animate-ui/icons/chevron-down';
import { ChevronRight as AnimatedChevronRight } from '@/components/animate-ui/icons/chevron-right';
import { ChevronUp as AnimatedChevronUp } from '@/components/animate-ui/icons/chevron-up';
import { CircleAlert as AnimatedCircleAlert } from '@/components/animate-ui/icons/circle-alert';
import { CircleCheck as AnimatedCircleCheck } from '@/components/animate-ui/icons/circle-check';
import { CircleQuestionMark as AnimatedCircleQuestionMark } from '@/components/animate-ui/icons/circle-question-mark';
import { Clock as AnimatedClock } from '@/components/animate-ui/icons/clock';
import { Clock3 as AnimatedClock3 } from '@/components/animate-ui/icons/clock-3';
import { Copy as AnimatedCopy } from '@/components/animate-ui/icons/copy';
import { Download as AnimatedDownload } from '@/components/animate-ui/icons/download';
import { Ellipsis as AnimatedEllipsis } from '@/components/animate-ui/icons/ellipsis';
import { Eye as AnimatedEye } from '@/components/animate-ui/icons/eye';
import { EyeOff as AnimatedEyeOff } from '@/components/animate-ui/icons/eye-off';
import { FileText as AnimatedFileText } from '@/components/animate-ui/icons/file-text';
import { Folder as AnimatedFolder } from '@/components/animate-ui/icons/folder';
import { Gift as AnimatedGift } from '@/components/animate-ui/icons/gift';
import { Key as AnimatedKey } from '@/components/animate-ui/icons/key';
import { KeyRound as AnimatedKeyRound } from '@/components/animate-ui/icons/key-round';
import { Layers as AnimatedLayers } from '@/components/animate-ui/icons/layers';
import { Link as AnimatedLink } from '@/components/animate-ui/icons/link';
import { Maximize as AnimatedMaximize } from '@/components/animate-ui/icons/maximize';
import { Megaphone as AnimatedMegaphone } from '@/components/animate-ui/icons/megaphone';
import { Minus as AnimatedMinus } from '@/components/animate-ui/icons/minus';
import { Pencil as AnimatedPencil } from '@/components/animate-ui/icons/pencil';
import { Plus as AnimatedPlus } from '@/components/animate-ui/icons/plus';
import { RefreshCw as AnimatedRefreshCw } from '@/components/animate-ui/icons/refresh-cw';
import { RotateCcw as AnimatedRotateCcw } from '@/components/animate-ui/icons/rotate-ccw';
import { Save as AnimatedSave } from '@/components/animate-ui/icons/save';
import { Search as AnimatedSearch } from '@/components/animate-ui/icons/search';
import { Send as AnimatedSend } from '@/components/animate-ui/icons/send';
import { SendHorizontal as AnimatedSendHorizontal } from '@/components/animate-ui/icons/send-horizontal';
import { ShieldCheck as AnimatedShieldCheck } from '@/components/animate-ui/icons/shield-check';
import { SlidersHorizontal as AnimatedSlidersHorizontal } from '@/components/animate-ui/icons/sliders-horizontal';
import { Star as AnimatedStar } from '@/components/animate-ui/icons/star';
import { SunMoon as AnimatedSunMoon } from '@/components/animate-ui/icons/sun-moon';
import { ExternalLink as AnimatedExternalLink } from '@/components/animate-ui/icons/external-link';
import { LogOut as AnimatedLogOut } from '@/components/animate-ui/icons/log-out';
import { Moon as AnimatedMoon } from '@/components/animate-ui/icons/moon';
import { Settings as AnimatedSettings } from '@/components/animate-ui/icons/settings';
import { Sun as AnimatedSun } from '@/components/animate-ui/icons/sun';
import { Share2 as AnimatedShare2 } from '@/components/animate-ui/icons/share-2';
import { CircleX as AnimatedCircleX } from '@/components/animate-ui/icons/circle-x';
import { List as AnimatedList } from '@/components/animate-ui/icons/list';
import { SlidersVertical as AnimatedSlidersVertical } from '@/components/animate-ui/icons/sliders-vertical';
import { Tag as AnimatedTag } from '@/components/animate-ui/icons/tag';
import { Unlink2 as AnimatedUnlink2 } from '@/components/animate-ui/icons/unlink-2';
import { Trash2 as AnimatedTrash2 } from '@/components/animate-ui/icons/trash-2';
import { TriangleAlert as AnimatedTriangleAlert } from '@/components/animate-ui/icons/triangle-alert';
import { Users as AnimatedUsers } from '@/components/animate-ui/icons/users';
import { WandSparkles as AnimatedWandSparkles } from '@/components/animate-ui/icons/wand-sparkles';
import { X as AnimatedX } from '@/components/animate-ui/icons/x';

/** Pixel size lucide-react icons render at when no size class applies. */
const LUCIDE_DEFAULT_SIZE = 24;

/** How long a navigation item counts as triggered after it becomes current, in milliseconds. */
const CURRENT_PULSE_MS = 120;

/** Whether the control around an icon is currently hovered or pressed. */
const IconTriggerContext = createContext(false);

/** Props accepted by every application icon, animated or still. */
export type AppIconProps = { className?: string; strokeWidth?: number } & AriaAttributes;

/** An icon component usable in navigation and section definitions. */
export type AppIcon = ComponentType<AppIconProps>;

/** An animated icon that can play on its own when it scrolls into view. */
export type AnimatedIcon = ComponentType<AppIconProps & { animateOnView?: boolean }>;

/**
 * Bind an Animate UI icon to the surrounding control: it plays while the control is hovered or
 * pressed and finishes its animation before returning to rest. Props such as `animateOnView`
 * start the animated icon right away and still apply on their own.
 *
 * Until its control is first triggered the icon renders as the matching lucide-react icon, so long
 * lists of idle icons cost no more than before; the first trigger swaps in Animate UI's animated
 * drawing of the same glyph.
 *
 * @param StillIcon - Matching lucide-react icon shown while the icon has never played.
 * @param Icon - Animate UI icon component.
 * @returns Icon that follows the nearest icon trigger.
 */
function withIconTrigger<Animation extends string>(
  StillIcon: LucideIcon,
  Icon: ComponentType<IconProps<Animation>>,
) {
  /**
   * Render the icon at lucide's default size and play it with its control.
   *
   * @param props - Icon properties.
   * @returns Still or animated icon.
   */
  function TriggeredIcon(props: IconProps<Animation>) {
    const isTriggered = use(IconTriggerContext);
    const [isArmed, setIsArmed] = useState(isTriggered);
    if (isTriggered && !isArmed) {
      setIsArmed(true);
    }
    const hasOwnAnimation =
      props.animate !== undefined ||
      props.animateOnHover !== undefined ||
      props.animateOnTap !== undefined ||
      props.animateOnView !== undefined;
    if (!isArmed && !isTriggered && !hasOwnAnimation) {
      const { className, strokeWidth, style, ...stillProps } = props;
      return (
        <StillIcon
          className={className as string | undefined}
          strokeWidth={strokeWidth as number | undefined}
          style={style as CSSProperties | undefined}
          {...(stillProps as AriaAttributes)}
        />
      );
    }
    return (
      <Icon
        size={LUCIDE_DEFAULT_SIZE}
        completeOnStop
        animate={hasOwnAnimation ? props.animate : isTriggered}
        {...props}
      />
    );
  }
  return TriggeredIcon;
}

type IconTriggerHandlers<Target extends Element> = {
  onPointerEnter?: (event: PointerEvent<Target>) => void;
  onPointerLeave?: (event: PointerEvent<Target>) => void;
  onPointerDown?: (event: PointerEvent<Target>) => void;
  onPointerUp?: (event: PointerEvent<Target>) => void;
  onPointerCancel?: (event: PointerEvent<Target>) => void;
};

/**
 * Track whether a control is hovered by a mouse or pressed by touch, keeping the control's own
 * pointer handlers. Navigation items also pass whether they are current: their icon plays once
 * each time the item becomes current after mounting.
 *
 * @param handlers - Pointer handlers the control already has.
 * @param isCurrent - Whether the control is the current navigation item.
 * @returns Trigger state and merged pointer handlers to spread onto the control.
 */
export function useIconTrigger<Target extends Element>(
  handlers: IconTriggerHandlers<Target>,
  isCurrent = false,
) {
  const [isHovered, setIsHovered] = useState(false);
  const [wasCurrent, setWasCurrent] = useState(isCurrent);
  const [isPulsing, setIsPulsing] = useState(false);
  if (isCurrent !== wasCurrent) {
    setWasCurrent(isCurrent);
    setIsPulsing(isCurrent);
  }
  useEffect(() => {
    if (!isPulsing) return;
    const timer = window.setTimeout(() => setIsPulsing(false), CURRENT_PULSE_MS);
    return () => window.clearTimeout(timer);
  }, [isPulsing]);
  const triggerHandlers = {
    onPointerEnter: (event: PointerEvent<Target>) => {
      handlers.onPointerEnter?.(event);
      if (event.pointerType !== 'touch') setIsHovered(true);
    },
    onPointerLeave: (event: PointerEvent<Target>) => {
      handlers.onPointerLeave?.(event);
      setIsHovered(false);
    },
    onPointerDown: (event: PointerEvent<Target>) => {
      handlers.onPointerDown?.(event);
      if (event.pointerType === 'touch') setIsHovered(true);
    },
    onPointerUp: (event: PointerEvent<Target>) => {
      handlers.onPointerUp?.(event);
      if (event.pointerType === 'touch') setIsHovered(false);
    },
    onPointerCancel: (event: PointerEvent<Target>) => {
      handlers.onPointerCancel?.(event);
      setIsHovered(false);
    },
  };
  return { isTriggered: isHovered || isPulsing, triggerHandlers };
}

/**
 * Let the animated icons inside a control play while the control is triggered.
 *
 * @param props - Trigger state and the control's content.
 * @returns Icon trigger boundary.
 */
export function IconTriggerProvider({
  isTriggered,
  children,
}: {
  isTriggered: boolean;
  children: ReactNode;
}) {
  return <IconTriggerContext value={isTriggered}>{children}</IconTriggerContext>;
}

export const AlertCircle = withIconTrigger(StillAlertCircle, AnimatedCircleAlert);
export const AlertTriangle = withIconTrigger(StillAlertTriangle, AnimatedTriangleAlert);
export const ArrowDown = withIconTrigger(StillArrowDown, AnimatedArrowDown);
export const ArrowLeft = withIconTrigger(StillArrowLeft, AnimatedArrowLeft);
export const ArrowUp = withIconTrigger(StillArrowUp, AnimatedArrowUp);
export const Bell = withIconTrigger(StillBell, AnimatedBell);
export const BellRing = withIconTrigger(StillBellRing, AnimatedBellRing);
export const BookOpen = withIconTrigger(StillBookOpen, AnimatedBookOpen);
export const Bookmark = withIconTrigger(StillBookmark, AnimatedBookmark);
export const BookmarkCheck = withIconTrigger(StillBookmarkCheck, AnimatedBookmarkCheck);
export const Bot = withIconTrigger(StillBot, AnimatedBot);
export const CalendarDays = withIconTrigger(StillCalendarDays, AnimatedCalendarDays);
export const Check = withIconTrigger(StillCheck, AnimatedCheck);
export const CheckCircle2 = withIconTrigger(StillCheckCircle2, AnimatedCircleCheck);
export const ChevronDown = withIconTrigger(StillChevronDown, AnimatedChevronDown);
export const ChevronRight = withIconTrigger(StillChevronRight, AnimatedChevronRight);
export const ChevronUp = withIconTrigger(StillChevronUp, AnimatedChevronUp);
export const CircleQuestionMark = withIconTrigger(
  StillCircleQuestionMark,
  AnimatedCircleQuestionMark,
);
export const CircleX = withIconTrigger(StillCircleX, AnimatedCircleX);
export const Clock = withIconTrigger(StillClock, AnimatedClock);
export const Clock3 = withIconTrigger(StillClock3, AnimatedClock3);
export const Copy = withIconTrigger(StillCopy, AnimatedCopy);
export const Download = withIconTrigger(StillDownload, AnimatedDownload);
export const ExternalLink = withIconTrigger(StillExternalLink, AnimatedExternalLink);
export const Eye = withIconTrigger(StillEye, AnimatedEye);
export const EyeOff = withIconTrigger(StillEyeOff, AnimatedEyeOff);
export const FileText = withIconTrigger(StillFileText, AnimatedFileText);
export const Folder = withIconTrigger(StillFolder, AnimatedFolder);
export const Gift = withIconTrigger(StillGift, AnimatedGift);
export const Key = withIconTrigger(StillKey, AnimatedKey);
export const KeyRound = withIconTrigger(StillKeyRound, AnimatedKeyRound);
export const Layers = withIconTrigger(StillLayers, AnimatedLayers);
export const LinkIcon = withIconTrigger(StillLinkIcon, AnimatedLink);
export const List = withIconTrigger(StillList, AnimatedList);
export const LogOut = withIconTrigger(StillLogOut, AnimatedLogOut);
export const Maximize = withIconTrigger(StillMaximize, AnimatedMaximize);
export const Megaphone = withIconTrigger(StillMegaphone, AnimatedMegaphone);
export const Minus = withIconTrigger(StillMinus, AnimatedMinus);
export const Moon = withIconTrigger(StillMoon, AnimatedMoon);
export const MoreHorizontal = withIconTrigger(StillMoreHorizontal, AnimatedEllipsis);
export const Pencil = withIconTrigger(StillPencil, AnimatedPencil);
export const Plus = withIconTrigger(StillPlus, AnimatedPlus);
export const RefreshCw = withIconTrigger(StillRefreshCw, AnimatedRefreshCw);
export const RotateCcw = withIconTrigger(StillRotateCcw, AnimatedRotateCcw);
export const Save = withIconTrigger(StillSave, AnimatedSave);
export const Search = withIconTrigger(StillSearch, AnimatedSearch);
export const Send = withIconTrigger(StillSend, AnimatedSend);
export const Settings = withIconTrigger(StillSettings, AnimatedSettings);
export const SendHorizontal = withIconTrigger(StillSendHorizontal, AnimatedSendHorizontal);
export const Share2 = withIconTrigger(StillShare2, AnimatedShare2);
export const ShieldCheck = withIconTrigger(StillShieldCheck, AnimatedShieldCheck);
export const SlidersHorizontal = withIconTrigger(StillSlidersHorizontal, AnimatedSlidersHorizontal);
export const SlidersVertical = withIconTrigger(StillSlidersVertical, AnimatedSlidersVertical);
export const Star = withIconTrigger(StillStar, AnimatedStar);
export const Sun = withIconTrigger(StillSun, AnimatedSun);
export const SunMoon = withIconTrigger(StillSunMoon, AnimatedSunMoon);
export const Tag = withIconTrigger(StillTag, AnimatedTag);
export const Trash2 = withIconTrigger(StillTrash2, AnimatedTrash2);
export const Unlink2 = withIconTrigger(StillUnlink2, AnimatedUnlink2);
export const Users = withIconTrigger(StillUsers, AnimatedUsers);
export const Wand2 = withIconTrigger(StillWand2, AnimatedWandSparkles);
export const X = withIconTrigger(StillX, AnimatedX);

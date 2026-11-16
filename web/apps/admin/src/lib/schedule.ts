/** 一次给几位候选人安排面试时，每个人的结果 */
export interface ScheduleOutcome<TPerson, TResult> {
  /** 安排成了的，按选的顺序 */
  done: { person: TPerson; result: TResult }[];
  /** 没成的，带着出错的原因 */
  failed: { person: TPerson; error: unknown }[];
}

/**
 * 给几位候选人各安排一场面试。一个一个来：后端每建一场就要开始出题，不一下子全压上去；
 * 中间有人没成不影响后面的人，最后一起告诉 HR 谁成了、谁没成。
 */
export async function scheduleAll<TPerson, TResult>(
  people: TPerson[],
  schedule: (person: TPerson) => Promise<TResult>,
  onProgress?: (finished: number, total: number) => void,
): Promise<ScheduleOutcome<TPerson, TResult>> {
  const outcome: ScheduleOutcome<TPerson, TResult> = { done: [], failed: [] };
  for (const [index, person] of people.entries()) {
    try {
      outcome.done.push({ person, result: await schedule(person) });
    } catch (error) {
      outcome.failed.push({ person, error });
    }
    onProgress?.(index + 1, people.length);
  }
  return outcome;
}

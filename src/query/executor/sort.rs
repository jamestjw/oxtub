use std::{cmp::Ordering, collections::VecDeque, iter::Peekable};

use crate::{
    buffer::{bpm::BufferPoolManager, page_guard::WritePageGuard},
    catalog::schema::Schema,
    common::types::PageId,
    query::{
        executor::{
            engine::ExecutorRow,
            error::ExecutionError,
            executor::{Executor, ExecutorContext},
            expression::{eval_comparison_is_true, generate_sort_key},
        },
        planner::{
            expression::ComparisonType,
            plan::{PlannedOrderBy, SortPlan},
        },
        statement::{OrderByNullType, OrderByType},
    },
    storage::{
        page::intermediate_result_page::{IntermediateResultPage, IntermediateResultPageMut},
        table::tuple::Tuple,
    },
    types::value::Value,
};

const SORT_CHILD_BATCH_SIZE: usize = 128;

/// The sort key defines the list of values that sorting is based on.
pub type SortKey = Vec<Value>;

/// A sort entry pairs the sort key with its source tuple.
pub type SortEntry = (SortKey, Tuple);

/// Compares sort keys and sort entries using the planned ORDER BY clauses.
pub struct TupleComparator {
    order_bys: Vec<PlannedOrderBy>,
}

impl TupleComparator {
    pub fn new(order_bys: Vec<PlannedOrderBy>) -> Self {
        Self { order_bys }
    }

    pub fn compare_keys(&self, key_a: &SortKey, key_b: &SortKey) -> Ordering {
        assert_eq!(key_a.len(), key_b.len());
        assert_eq!(key_a.len(), self.order_bys.len());

        for ((v1, v2), order_by) in key_a.iter().zip(key_b.iter()).zip(self.order_bys.iter()) {
            match (v1.is_null(), v2.is_null(), order_by.null_type) {
                (true, true, _) => continue,
                (true, false, OrderByNullType::First) => return Ordering::Less,
                (true, false, OrderByNullType::Last) => return Ordering::Greater,
                (false, true, OrderByNullType::First) => return Ordering::Greater,
                (false, true, OrderByNullType::Last) => return Ordering::Less,
                (false, false, _) => (),
            }

            // TODO: this clone should not be necessary?
            if eval_comparison_is_true(v1.clone(), v2.clone(), &ComparisonType::Eq).unwrap() {
                continue;
            }

            if eval_comparison_is_true(v1.clone(), v2.clone(), &ComparisonType::LessThan).unwrap() {
                return match order_by.order_by_type {
                    OrderByType::Asc => Ordering::Less,
                    OrderByType::Desc => Ordering::Greater,
                };
            } else {
                return match order_by.order_by_type {
                    OrderByType::Asc => Ordering::Greater,
                    OrderByType::Desc => Ordering::Less,
                };
            }
        }

        Ordering::Equal
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        catalog::types::SqlType,
        query::planner::expression::{
            ConstantValueExpression, ExpressionType, PlannedExpression, PlannedExpressionKind,
        },
    };

    fn order_by(order_by_type: OrderByType, null_type: OrderByNullType) -> PlannedOrderBy {
        PlannedOrderBy {
            expression: PlannedExpression {
                return_type: ExpressionType {
                    sql_type: SqlType::Integer,
                    varchar_size: None,
                },
                kind: PlannedExpressionKind::ConstantValue(ConstantValueExpression {
                    value: Value::Integer(0),
                }),
            },
            order_by_type,
            null_type,
        }
    }

    #[test]
    fn compares_ascending_and_descending_values() {
        let ascending =
            TupleComparator::new(vec![order_by(OrderByType::Asc, OrderByNullType::Last)]);
        assert_eq!(
            ascending.compare_keys(&vec![Value::Integer(1)], &vec![Value::Integer(2)]),
            Ordering::Less
        );
        assert_eq!(
            ascending.compare_keys(&vec![Value::Integer(2)], &vec![Value::Integer(1)]),
            Ordering::Greater
        );

        let descending =
            TupleComparator::new(vec![order_by(OrderByType::Desc, OrderByNullType::Last)]);
        assert_eq!(
            descending.compare_keys(&vec![Value::Integer(1)], &vec![Value::Integer(2)]),
            Ordering::Greater
        );
        assert_eq!(
            descending.compare_keys(&vec![Value::Integer(2)], &vec![Value::Integer(1)]),
            Ordering::Less
        );
    }

    #[test]
    fn compares_multiple_keys_lexicographically() {
        let comparator = TupleComparator::new(vec![
            order_by(OrderByType::Asc, OrderByNullType::Last),
            order_by(OrderByType::Desc, OrderByNullType::Last),
        ]);

        assert_eq!(
            comparator.compare_keys(
                &vec![Value::Integer(1), Value::Integer(2)],
                &vec![Value::Integer(1), Value::Integer(3)],
            ),
            Ordering::Greater
        );
        assert_eq!(
            comparator.compare_keys(
                &vec![Value::Integer(1), Value::Integer(2)],
                &vec![Value::Integer(2), Value::Integer(1)],
            ),
            Ordering::Less
        );
    }

    #[test]
    fn compares_nulls_using_null_order() {
        let nulls_first =
            TupleComparator::new(vec![order_by(OrderByType::Desc, OrderByNullType::First)]);
        assert_eq!(
            nulls_first.compare_keys(
                &vec![Value::Null(SqlType::Integer)],
                &vec![Value::Integer(1)],
            ),
            Ordering::Less
        );

        let nulls_last =
            TupleComparator::new(vec![order_by(OrderByType::Asc, OrderByNullType::Last)]);
        assert_eq!(
            nulls_last.compare_keys(
                &vec![Value::Null(SqlType::Integer)],
                &vec![Value::Integer(1)],
            ),
            Ordering::Greater
        );
        assert_eq!(
            nulls_last.compare_keys(
                &vec![Value::Null(SqlType::Integer)],
                &vec![Value::Null(SqlType::Integer)],
            ),
            Ordering::Equal
        );
    }
}

pub struct MergeSortRun<'bpm> {
    bpm: &'bpm BufferPoolManager,
    page_ids: Vec<PageId>,
}

impl<'bpm> MergeSortRun<'bpm> {
    pub fn new(bpm: &'bpm BufferPoolManager, page_ids: Vec<PageId>) -> Self {
        Self { bpm, page_ids }
    }
}

impl<'bpm> IntoIterator for MergeSortRun<'bpm> {
    type Item = Result<Tuple, ExecutionError>;
    type IntoIter = MergeSortRunIterator<'bpm>;

    fn into_iter(self) -> Self::IntoIter {
        MergeSortRunIterator::new(self.bpm, self.page_ids)
    }
}

pub struct MergeSortRunIterator<'bpm> {
    bpm: &'bpm BufferPoolManager,
    page_ids: Vec<PageId>,
    page_idx: usize,
    tuple_idx: usize,
}

impl<'bpm> MergeSortRunIterator<'bpm> {
    pub fn new(bpm: &'bpm BufferPoolManager, page_ids: Vec<PageId>) -> Self {
        Self {
            bpm,
            page_ids,
            page_idx: 0,
            tuple_idx: 0,
        }
    }
}

impl Iterator for MergeSortRunIterator<'_> {
    type Item = Result<Tuple, ExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.page_idx >= self.page_ids.len() {
            return None;
        }

        let curr_page_id = self.page_ids[self.page_idx];
        let page_guard = match self.bpm.read_page(curr_page_id) {
            Ok(page_guard) => page_guard,
            Err(err) => return Some(Err(err.into())),
        };
        let page = IntermediateResultPage::from_data(page_guard.page_data());

        if self.tuple_idx >= page.num_tuples() {
            self.tuple_idx = 0;
            drop(page_guard);
            if let Err(e) = self.bpm.delete_page(curr_page_id) {
                return Some(Err(e.into()));
            }
            self.page_idx += 1;
            return self.next();
        }

        let tuple = page.get_tuple(self.tuple_idx);
        self.tuple_idx += 1;

        Some(Ok(tuple))
    }
}

impl Drop for MergeSortRunIterator<'_> {
    fn drop(&mut self) {
        // Cleanup in case we don't consume all the pages
        for page_id in &self.page_ids[self.page_idx..] {
            if let Err(err) = self.bpm.delete_page(*page_id) {
                tracing::warn!(page_id, error = %err, "failed to delete sort run page");
            }
        }
    }
}

fn merge_sorted_runs<'a, 'bpm>(
    mut a: Peekable<impl Iterator<Item = Result<SortEntry, ExecutionError>> + 'a>,
    mut b: Peekable<impl Iterator<Item = Result<SortEntry, ExecutionError>> + 'a>,
    comparator: &'a TupleComparator,
) -> impl Iterator<Item = Result<SortEntry, ExecutionError>> + 'a {
    std::iter::from_fn(move || match (a.peek(), b.peek()) {
        (Some(Ok((x_key, _))), Some(Ok((y_key, _)))) => {
            if comparator.compare_keys(x_key, y_key) != Ordering::Greater {
                a.next()
            } else {
                b.next()
            }
        }
        (Some(Err(_)), _) => a.next(),
        (_, Some(Err(_))) => b.next(),
        (Some(_), None) => a.next(),
        (None, Some(_)) => b.next(),
        (None, None) => None,
    })
}

pub struct ExternalMergeSortExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan SortPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
    comparator: TupleComparator,
    sorted_iterator: Option<MergeSortRunIterator<'bpm>>,
}

impl<'ctx, 'catalog, 'bpm, 'plan> ExternalMergeSortExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    pub fn new(
        exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
        plan: &'plan SortPlan,
        output_schema: &'plan Schema,
        child: Box<dyn Executor + 'plan>,
    ) -> Self {
        Self {
            exec_ctx,
            plan,
            output_schema,
            child,
            comparator: TupleComparator::new(plan.order_bys.clone()),
            sorted_iterator: None,
        }
    }

    fn insert_into_page(
        &self,
        mut page_guard: WritePageGuard<'bpm>,
        tuple: Tuple,
    ) -> Result<(WritePageGuard<'bpm>, Option<PageId>), ExecutionError> {
        let mut page = IntermediateResultPageMut::from_data(page_guard.page_data_mut());
        match page.insert_tuple(&tuple) {
            Some(_) => Ok((page_guard, None)),
            None => {
                // not enough space in curr page
                drop(page_guard);
                let new_page_id = self.exec_ctx.bpm().new_page();
                let mut new_page_guard = self.exec_ctx.bpm().write_page(new_page_id)?;
                let mut page =
                    IntermediateResultPageMut::init_from_data(new_page_guard.page_data_mut());

                match page.insert_tuple(&tuple) {
                    Some(_) => Ok((new_page_guard, Some(new_page_id))),
                    None => Err(ExecutionError::TupleTooBig),
                }
            }
        }
    }
}

impl Executor for ExternalMergeSortExecutor<'_, '_, '_, '_> {
    fn init(&mut self) -> Result<(), ExecutionError> {
        self.child.init()?;

        let child_schema = self.child.output_schema().clone();
        let curr_page_id = self.exec_ctx.bpm().new_page();
        let mut curr_page_guard = self.exec_ctx.bpm().write_page(curr_page_id)?;
        let mut unsorted_pages: Vec<PageId> = vec![curr_page_id];

        loop {
            let batch = self.child.next(SORT_CHILD_BATCH_SIZE)?;

            if batch.is_empty() {
                break;
            }

            for tuple in batch.into_iter() {
                let (page_guard, new_page_id) = self.insert_into_page(
                    curr_page_guard,
                    Tuple::from_values(&tuple.values, &child_schema),
                )?;
                curr_page_guard = page_guard;

                if let Some(new_page_id) = new_page_id {
                    unsorted_pages.push(new_page_id);
                }
            }
        }
        // It's possible that unsorted_pages contains only a single page and that it's empty
        // the rest of the code should be able to handle it though
        drop(curr_page_guard);

        // Build initial runs of sorted pages
        let mut current_runs: VecDeque<Vec<PageId>> = VecDeque::new();

        for page_id in unsorted_pages {
            let mut unsorted_page_guard = self.exec_ctx.bpm().write_page(page_id)?;
            let mut unsorted_page =
                IntermediateResultPageMut::from_data(unsorted_page_guard.page_data_mut());

            let mut sorted_entries: Vec<SortEntry> = unsorted_page
                .all_tuples()
                .into_iter()
                .map(|tuple| {
                    Ok((
                        generate_sort_key(&tuple, &child_schema, &self.plan.order_bys)?,
                        tuple,
                    ))
                })
                .collect::<Result<Vec<_>, ExecutionError>>()?;
            sorted_entries
                .sort_by(|(key_0, _), (key_1, _)| self.comparator.compare_keys(key_0, key_1));

            // Reinitialise the page as we are re-using it for the sorted tuples
            unsorted_page.init();

            for (_, tuple) in sorted_entries {
                unsorted_page
                    .insert_tuple(&tuple)
                    .expect("if it came from the page, it should still fit now");
            }

            current_runs.push_back(vec![page_id]);
        }

        // We need to keep going until we have merged all runs into a single one
        while current_runs.len() > 1 {
            let mut next_runs: VecDeque<Vec<PageId>> = VecDeque::new();

            // Take two runs at a time, and make a single longer run of sorted stuff
            while !current_runs.is_empty() {
                if current_runs.len() == 1 {
                    next_runs.push_back(current_runs.pop_back().unwrap());
                } else {
                    // Both of these runs are already sorted, so we just need to merge them
                    let run_page_ids_1 = current_runs.pop_front().unwrap();
                    let run_page_ids_2 = current_runs.pop_front().unwrap();

                    let run_1 = MergeSortRun::new(self.exec_ctx.bpm(), run_page_ids_1);
                    let run_2 = MergeSortRun::new(self.exec_ctx.bpm(), run_page_ids_2);

                    let curr_page_id = self.exec_ctx.bpm().new_page();
                    let mut curr_page_guard = self.exec_ctx.bpm().write_page(curr_page_id)?;
                    let mut merged_page_ids: Vec<PageId> = vec![curr_page_id];

                    let run_1_entries = run_1.into_iter().map(|tuple| {
                        let tuple = tuple?;
                        let sort_key =
                            generate_sort_key(&tuple, &child_schema, &self.plan.order_bys)?;
                        Ok((sort_key, tuple))
                    });
                    let run_2_entries = run_2.into_iter().map(|tuple| {
                        let tuple = tuple?;
                        let sort_key =
                            generate_sort_key(&tuple, &child_schema, &self.plan.order_bys)?;
                        Ok((sort_key, tuple))
                    });

                    for entry in merge_sorted_runs(
                        run_1_entries.peekable(),
                        run_2_entries.peekable(),
                        &self.comparator,
                    ) {
                        let (_, tuple) = entry?;
                        let (page_guard, new_page_id) =
                            self.insert_into_page(curr_page_guard, tuple)?;
                        curr_page_guard = page_guard;

                        if let Some(new_page_id) = new_page_id {
                            merged_page_ids.push(new_page_id);
                        }
                    }

                    next_runs.push_back(merged_page_ids);
                }
            }
            current_runs = next_runs;
        }

        let final_sorted_run =
            MergeSortRun::new(self.exec_ctx.bpm(), current_runs.pop_back().unwrap());

        self.sorted_iterator = Some(final_sorted_run.into_iter());

        Ok(())
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        match self.sorted_iterator.as_mut() {
            None => Err(ExecutionError::Uninitialised),
            Some(it) => {
                (it.take(batch_size)
                    .map(|tuple| {
                        Ok(ExecutorRow {
                            rid: None,
                            values: tuple?.get_values(self.output_schema),
                        })
                    })
                    .collect())
            }
        }
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}

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
            expression::generate_sort_key,
        },
        planner::plan::{PlannedOrderBy, SortPlan},
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
        todo!("compare sort keys")
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

    pub fn page_ids(&self) -> &[PageId] {
        &self.page_ids
    }

    pub fn into_iter(self) -> MergeSortRunIterator<'bpm> {
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
    mut a: Peekable<MergeSortRunIterator<'bpm>>,
    mut b: Peekable<MergeSortRunIterator<'bpm>>,
    comparator: &'a TupleComparator,
    schema: &'a Schema,
    order_bys: &'a [PlannedOrderBy],
) -> impl Iterator<Item = Result<Tuple, ExecutionError>> + 'a
where
    'bpm: 'a,
{
    std::iter::from_fn(move || match (a.peek(), b.peek()) {
        (Some(Ok(x)), Some(Ok(y))) => {
            let x_key = match generate_sort_key(x, schema, order_bys) {
                Ok(key) => key,
                Err(err) => {
                    a.next();
                    return Some(Err(err));
                }
            };
            let y_key = match generate_sort_key(y, schema, order_bys) {
                Ok(key) => key,
                Err(err) => {
                    b.next();
                    return Some(Err(err));
                }
            };

            if comparator.compare_keys(&x_key, &y_key) != Ordering::Greater {
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
    sorted_page_ids: Vec<PageId>,
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
            sorted_page_ids: Vec::new(),
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

                    for tuple in merge_sorted_runs(
                        run_1.into_iter().peekable(),
                        run_2.into_iter().peekable(),
                        &self.comparator,
                        &child_schema,
                        &self.plan.order_bys,
                    ) {
                        let (page_guard, new_page_id) =
                            self.insert_into_page(curr_page_guard, tuple?)?;
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

        let final_sorted_run: MergeSortRun<'_> =
            MergeSortRun::new(self.exec_ctx.bpm(), current_runs.pop_back().unwrap());

        // TODO: see if we end up using this field or not, might be superfluous
        self.sorted_page_ids = final_sorted_run.page_ids().to_vec();
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

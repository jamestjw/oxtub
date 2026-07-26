use std::{cmp::Ordering, collections::VecDeque};

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

    // TODO: dont think this is needed right?
    // pub fn compare_entries(&self, entry_a: &SortEntry, entry_b: &SortEntry) -> Ordering {
    //     todo!("compare sort entries")
    // }

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

        let page_guard = match self.bpm.read_page(self.page_ids[self.page_idx]) {
            Ok(page_guard) => page_guard,
            Err(err) => return Some(Err(err.into())),
        };
        let page = IntermediateResultPage::from_data(page_guard.page_data());

        if self.tuple_idx >= page.num_tuples() {
            self.tuple_idx = 0;
            self.page_idx += 1;
            return self.next();
        }

        let tuple = page.get_tuple(self.tuple_idx);
        self.tuple_idx += 1;

        Some(Ok(tuple))
    }
}

pub struct ExternalMergeSortExecutor<'ctx, 'catalog, 'bpm, 'plan> {
    exec_ctx: &'ctx ExecutorContext<'catalog, 'bpm>,
    plan: &'plan SortPlan,
    output_schema: &'plan Schema,
    child: Box<dyn Executor + 'plan>,
    comparator: TupleComparator,
    sorted_page_ids: Vec<PageId>,
    sorted_iterator: Option<MergeSortRunIterator<'bpm>>,
    dropped_final_run: bool,
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
            dropped_final_run: false,
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
            todo!("finish this")
        }

        let final_sorted_run: MergeSortRun<'_> =
            MergeSortRun::new(self.exec_ctx.bpm(), current_runs.pop_back().unwrap());

        // TODO: see if we end up using this field or not, might be superfluous
        self.sorted_page_ids = final_sorted_run.page_ids().to_vec();
        self.sorted_iterator = Some(final_sorted_run.into_iter());

        Ok(())
    }

    fn next(&mut self, batch_size: usize) -> Result<Vec<ExecutorRow>, ExecutionError> {
        todo!("pull sorted tuples from sorted_iterator")
    }

    fn output_schema(&self) -> &Schema {
        self.output_schema
    }
}
